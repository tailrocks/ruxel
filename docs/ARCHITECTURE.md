# ARCHITECTURE — How Ruxel Works

Status: detailed concept, 2026-06-11. Companion to
[SEMANTICS.md](SEMANTICS.md) (what must be reproduced) and
[PLAN.md](PLAN.md) (build order). Scope rule inherited from the operator:
**implement only the surface in SEMANTICS.md — the features, parameters,
and values these playbooks use. Nothing else.** Unknown module, unknown
parameter, unknown value → hard error at parse time, never silent
acceptance.

---

## 1. Shape of the system

```
controller (operator laptop)                    target host (Debian 12, root)
┌──────────────────────────────────┐           ┌─────────────────────────────────┐
│ ruxel CLI                        │           │ ruxel-agent                     │
│                                  │           │  static x86_64-musl binary      │
│ inventory + playbook parser      │           │  /var/lib/ruxel/agent/<b3sum>   │
│ MiniJinja renderer (all Jinja    │  SSH      │                                 │
│   stays controller-side)         │  conn     │  module runtime (native Rust)   │
│ secret resolver (op, ‖, memoized)│  ──────▶  │  system caches (dpkg snapshot,  │
│ plan compiler (loop expansion,   │  ch0:     │    sd-bus conn, pg conn, mounts)│
│   register-dependency DAG)       │  protocol │  probe engine (‖)               │
│ scheduler / register pipeline    │  ch1:     │  convergence ledger             │
│ differ + reporter (tty/json)     │  SFTP     │    /var/lib/ruxel/ledger        │
└──────────────────────────────────┘  blobs    └─────────────────────────────────┘
        hosts run in parallel; per host: ONE ssh connection for the whole run
```

Two binaries, one repo, one protocol crate shared between them. The agent
contains **no template engine, no YAML parser, no secret store** — it
receives fully rendered, typed task structs and executes them.

## 2. Transport: the decision

Question asked: is SSH right at all? gRPC over SSH? Something else?

### Options considered

| # | Option | Verdict |
|---|---|---|
| A | SSH exec per task (Ansible's model) | Rejected — the per-task session+upload tax is the problem being solved. |
| B | **SSH as carrier + resident agent per run + framed binary protocol over stdio** | **Chosen.** One connection, one process, everything streamed. |
| C | gRPC (tonic/h2) tunneled over an SSH channel | Rejected as framing: h2's stream multiplexing is redundant on a private 1:1 pipe (SSH already multiplexes channels), and it inserts an HTTP/2 state machine between us and the bytes. **But its schema discipline is kept** — see "gRPC minus the g" below. |
| D | Standalone agent daemon with its own listener (gRPC/QUIC + mTLS) | Rejected for v1 on correctness grounds: a new open port, a CA, cert issuance/rotation, and firewall changes are new state that can drift and a second auth system to keep correct — on servers whose entire access model today is one SSH key. Revisited only as the optional warm-daemon tier (§9), which still rides SSH. |
| E | Compile-to-shell, no agent (pyinfra/glidesh model) | Rejected: shell output parsing is a weaker correctness foundation than typed native checks; no place to keep the ledger logic; loses batched system caches. |

### Why SSH stays (as carrier, not as executor)

- **Auth is already solved and audited**: the exact root keys, `~/.ssh`
  config, and host-key trust the operator uses today. Zero new attack
  surface, zero new credentials, zero firewall changes.
- **One connection is enough**: a single SSH connection natively
  multiplexes channels — ruxel uses channel 0 for the protocol stream and a
  second SFTP channel for content-addressed blob transfer, concurrently.
  Post-handshake throughput (aes-gcm) is far beyond anything a config run
  moves.
- **Bootstrap is free**: the same connection uploads the agent binary the
  first time (and never again until the version hash changes).

Implementation: the `openssh` crate over the system OpenSSH with
**ControlMaster native-mux** — operator's `~/.ssh/config`, agent auth, and
known_hosts behave byte-identically to their `ansible-playbook` runs today.
The transport sits behind a small trait; pure-Rust `russh` is the swap-in
if controlling the SSH stack ever becomes necessary (capability choice, not
a now-decision).

### The protocol: "gRPC minus the g"

Messages are defined in **protobuf** (`proto/ruxel.proto`, compiled with
prost) — schema'd, versioned, evolvable — but framed directly on the SSH
stdio stream as `varint length ‖ message bytes`. No HTTP/2, no TLS-in-TLS,
no port. What gRPC would have provided (typed contract, streaming,
versioning) is kept; what it would have cost (h2 framing/flow-control on a
pipe that already has both) is dropped. If the warm-daemon tier ever wants
a network protocol, the same `.proto` lifts into tonic unchanged.

Message flow per host per run:

```
controller → agent   Hello{proto_ver, agent_b3sum_expected, run_id, flags(check, diff, no_cache)}
agent → controller   HelloAck{agent_ver, facts{default_ipv4_iface, distro_release, arch}, ledger_gen}
controller → agent   Plan{tasks: [RenderedTask], handlers: [...], blobs_referenced: [b3sums]}
agent → controller   BlobsNeeded{missing: [b3sums]}          // controller pushes via SFTP channel
agent → controller   stream of Event:
                       ProbeResult{task_id, verdict: CachedOk | NeedsCheck | NeedsApply, fingerprint_diff}
                       TaskStart{task_id, item_label?}
                       TaskResult{task_id, status, changed, rc/stdout/stderr?, diff?, timing, register_payload}
                       PauseRequest{prompt}                  // controller relays to operator TTY, replies Resume
                       Log{level, msg}
controller → agent   PlanPatch{tasks: [...]}                 // register-dependent tasks rendered late (§4)
controller → agent   Done → agent flushes ledger, exits      // (ephemeral mode)
```

`register_payload` carries the full result dict (rc, stdout, stat fields,
…) so the controller can render dependent expressions — all Jinja stays on
the controller.

## 3. Execution pipeline (one run, end to end)

1. **Parse** inventory + playbook into the typed model (SEMANTICS §1–§4).
   Hard error on anything outside the closed surface.
2. **Resolve secrets**: collect every distinct `onepassword`/`pipe` lookup
   across the effective tasks; **group lookups by 1Password item** (an SSH
   item's private+public key = two lookups, one `op item get <item>
   --format json` fetch) and fetch all distinct items concurrently through
   one `op` session; memoize for the run. (Specified deviation, SEMANTICS
   §2.) The 52 lookups collapse to roughly half as many item fetches.
3. **Compile**: evaluate statically renderable expressions; expand loops
   whose source is already known (literal lists, play-var lists); build the
   **register-dependency DAG** — each task is annotated with the registered
   vars it reads (`when`, `loop`, params, `until`). Tasks whose inputs are
   all static are fully rendered now; the rest become *deferred nodes*
   rendered when their inputs arrive (§4).
4. **Connect** to all target hosts in parallel; handshake; upload agent if
   hash-missing; send `Plan` (ready tasks + deferred placeholders so the
   agent knows the full shape and ordering).
5. **Probe phase** (the speed heart, §6): agent concurrently evaluates
   ledger fingerprints for every probeable task and streams verdicts.
6. **Plan output / apply**: in `plan` mode, render the diff and stop. In
   `apply` mode, agent executes the not-verified tasks **in playbook
   order**, streaming results; the controller renders deferred nodes as
   their register inputs arrive and streams `PlanPatch` continuations —
   the agent never waits idle unless a true data dependency forces it.
7. **Handlers** flush at end of play in definition order (only notified +
   changed, SEMANTICS §4).
8. **Ledger update** (per successful task), recap with per-task timing,
   exit code.

## 4. Register-dependency pipelining

The workload's pattern: `stat` a set of disks → `register` → `when`/`loop`
over results → `readlink` → `register` → LVM tasks templated from that.
Ansible handles this by being fully sequential. Ruxel keeps **templating on
the controller** (single Jinja implementation, single truth) without
round-trip stalls becoming the bottleneck:

- The compiler splits the task list into **issue windows**: maximal runs of
  consecutive tasks whose params/conditions are already rendered.
- Window N is streaming results while the controller renders window N+1
  from arriving `register_payload`s. The added latency per dependency edge
  is one controller round-trip (~RTT, tens of ms to Hetzner) — paid only at
  true data dependencies, of which the playbooks have a handful, not 65.
- `assert`, `set_fact`, `debug`, `fail` and `when`-only evaluation execute
  entirely on the controller (no agent round-trip at all).

## 5. The agent: native modules over batched system caches

One process per run (or resident, §9), tokio runtime, panic=abort with a
structured crash report event. Module implementations follow SEMANTICS §6
exactly. The performance design is **shared system snapshots** so that "65
tasks" stops meaning "65 interrogations":

| Cache | Built | Serves |
|---|---|---|
| dpkg status snapshot (one parse of `/var/lib/dpkg/status` + `apt-cache policy` batch for `latest`) | once per run, invalidated on any apt write | all 24 `apt` + 6 `apt_repository` checks |
| systemd: one D-Bus connection, `ListUnits`+`unit file state` batch | once | all 21 `systemd` + 8 `service` checks; `daemon_reload` coalesced to ≤1 per run window |
| PostgreSQL: one connection (unix socket, peer auth as postgres, port 40000) | lazily | all 44 `postgresql_*` checks/changes |
| `/proc/mounts` + `blkid` + `vgs/lvs --reportformat json` snapshot | once, invalidated on storage writes | mount/lvg/lvol/filesystem |
| `getent passwd/group` snapshot | once, invalidated on user/group writes | user/group/authorized_key |
| `iptables-save` parse | once per table, invalidated on writes | all 8 iptables checks |
| blob store `/var/lib/ruxel/blobs/<b3sum>` | content-addressed, persistent | copy/template payloads — a file already delivered is never re-sent |

**Apt batching rule** (the only cross-task merge, and it is
status-preserving): a maximal run of *consecutive* `apt state=present`
tasks with no intervening register consumption, `when` dependency, or
non-apt task collapses into one `apt-get install` transaction; per-task
changed/ok status is reconstructed from the dpkg snapshot delta so
reporting and `notify` behave exactly as if run singly. No other module is
cross-task merged in v1.

Shell/command tasks run exactly as written (`/bin/bash -c`, `chdir`,
`creates` guard, merged `environment`), streamed rc/stdout/stderr.

## 6. The convergence ledger (why no-op is seconds)

Per-host store: `/var/lib/ruxel/ledger/` — an append-compacted redb (or
equivalent single-writer) keyed by **task identity**:

```
task_id  = blake3(playbook_rel_path ‖ play_name ‖ task_name ‖ module ‖ canonical(params))
           where every secret-derived param value is replaced by
           HMAC(host_ledger_key, value) — identity changes when a secret
           changes, but no recoverable secret material is ever stored.
record   = { agent_version, module_version, completed_at,
             fingerprints: [Probe], last_status }
Probe    = File{path, b3sum, len, mtime} | Pkg{name, version} |
           UnitState{name, active, enabled, unitfile_b3} |
           Mount{path, src_uuid, fstype, opts_norm} | SysctlKV |
           VG{name, pv_ids} | LV{vg, name} | FsOnDev{dev, fstype} |
           PgObject{kind, db, name, owner_or_acl_hash} |
           PasswdEnt | GroupEnt | IptablesRule{table, chain, spec_hash} |
           GitHead{dest, sha} | PathExists{path} | Marker{key}
```

Run-time verdict per task, computed concurrently for the whole plan:

1. `task_id` present + **all fingerprints re-verify** (a few hundred stats
   /reads ≈ <0.5 s total) → `CachedOk` — module logic not even invoked.
2. Fingerprint mismatch (= drift) or unknown `task_id` (= edited task) →
   full native check (SEMANTICS §6) → no-op or apply, and re-record.
3. Ledger absent/corrupt/agent-version-changed → class 2 for everything
   (graceful degradation: still no Python, still batched — minutes faster
   than Ansible, just not instant). `--no-cache` forces class 2 globally.

**Probe-class table** — every task type is explicitly classed; nothing
defaults silently to "trust":

| Class | Task types | No-op cost |
|---|---|---|
| Fingerprintable | file/copy/template/lineinfile/replace/blockinfile, apt, apt_repository, systemd/service *state=started,enabled*, sysctl, mount/lvg/lvol/filesystem, user/group/authorized_key, iptables, postgresql_*, timezone, git (HEAD sha + clean check) | µs–ms each, all parallel |
| Always-execute | `systemd/service state=restarted` (an action), shell/command **without** `creates` (incl. the 26 `changed_when: false` check-commands — they *are* verification and still run, in parallel where the register DAG allows), `until` waits, pause, assert/fail/debug/set_fact (controller-side, ~free) | bounded by the commands themselves |
| Guarded | shell with `creates` → `PathExists` probe short-circuits | µs |
| Network-truth | `apt update_cache`, `apt state=latest`, `git update=yes`, `get_url` with missing dest | one network op each, parallelized |

Honesty rule: a fingerprint match never overrides a *mandatory-execute*
semantic. The ledger accelerates state checks; it never suppresses actions
the playbook says happen every run.

## 7. plan / apply / check / diff / tags / limit

- `ruxel plan …` ≡ trustworthy `--check --diff` in seconds: probe phase +
  class-2 checks only, **plus** honest annotation of what cannot be
  predicted (shell tasks without `creates` report "would run", matching
  Ansible's check-mode skip semantics for command/shell — SEMANTICS §3.5).
- `ruxel apply …` = the full pipeline. `--check`/`--diff` are accepted as
  aliases of plan behavior for drop-in muscle memory.
- `--limit`, `--tags` (incl. `always`) behave per SEMANTICS §4. Tags keep
  working — they just stop being a performance necessity.
- Exit codes: 0 success, 1 any host failed, 2 usage/parse error;
  `--detailed-exitcode` (opt-in) adds terraform-style "0 = converged,
  2 = changes were applied/needed".
- Output: ansible-shaped task lines with per-task timing and verdict
  annotation (`ok (ledger)`, `ok (checked)`, `changed`, `would-run`),
  recap table, `--output json` = stable JSON-lines event stream (the same
  protocol events, serialized).
- **Run log**: every run additionally writes its full JSON event stream
  (secrets redacted) to `~/.local/state/ruxel/runs/<timestamp>-<run_id>.jsonl`
  — forensics ("what exactly changed last Tuesday"), timing history, and
  the raw material for future drift dashboards. Pruned by count, never a
  dependency of execution.
- Inventory vs `~/.ssh/config` precedence: `ansible_ssh_host`/
  `ansible_ssh_user` from `hosts.ini` always win (passed explicitly to the
  connection); everything else (keys, agent, ciphers, ControlMaster paths)
  comes from the operator's ssh config — exactly the effective behavior of
  their `ansible-playbook` runs today.

## 8. Failure, interruption, safety

- Task failure: stop that host (no rescue) / jump to rescue (in block);
  other hosts continue. Identical to today's semantics.
- Connection loss / controller Ctrl-C: agent finishes the in-flight task
  (never kills a half-applied module action mid-write), writes the ledger,
  exits. Rerun converges — every module impl is re-entrant by the same
  idempotence rules it checks with.
- The agent refuses to run two overlapping runs (flock on the ledger).
- Secrets: exist only in controller memory and in rendered params streamed
  over SSH; never written to target disk (template/copy payload bytes
  excepted — that *is* the operator's intent); `no_log` redacts protocol
  logging, diffs, JSON output, and ledger identity hashing (§6).
- Production-safety rule ([AGENTS.md](../AGENTS.md)) unchanged: all
  development against disposable VMs; the tool grows up before it ever
  sees a production host, and then only operator-driven.

## 9. Warm-daemon tier (designed now, built later)

The ephemeral agent already meets the seconds target. The same binary
gains `ruxel-agent --resident`: stays up under a systemd unit (installed
by ruxel itself like any other unit), holds the parsed system snapshots
and ledger in memory, watches fingerprinted paths with inotify, and
listens on a **unix socket only** — the controller reaches it through SSH
unix-socket forwarding (`direct-streamlocal`), so the no-new-listener
security posture is preserved. Effect: connect → verdicts in one RTT, and
proactive drift reporting becomes possible (push to Slack/Sentry).
Protocol identical (§2). This is the path to "I glance at a dashboard and
already know everything is correct."

## 10. Performance budget (converged `setup-postgresql-nova.yml`, 65 tasks)

| Step | Cost |
|---|---|
| ControlMaster connect + channel + agent spawn | ~0.3–0.5 s |
| Parse + compile (local) | ~50 ms |
| Secret resolution (52 lookups, deduplicated, ‖, warm `op` session) | ~1–3 s |
| Plan stream + ledger probes (‖) + mandatory check-commands (‖ where DAG allows) | ~0.5–1.5 s |
| Render + recap | ~50 ms |
| **Total** | **≈ 2–5 s** (today: ~15 min) |

Edit-one-task rerun = the same + that one task's real work. Fresh-server
first run = real work (apt/mkfs/initdb) + seconds of overhead, hosts in
parallel. These budgets become measured benchmarks in M3/M5
([PLAN.md](PLAN.md)); the numbers above are targets, not claims.

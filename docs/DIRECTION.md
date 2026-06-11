# Direction — Analysis and Recommended Architecture

Status: proposal for operator review. 2026-06-11.
Follow-up: [SKEPTIC.md](SKEPTIC.md) deliberately attacks this document's
conclusion with churn evidence, the untuned-Ansible finding, and NixOS/Tofu
deep-dives, then re-derives a staged recommendation (tune → unbundle →
verify-first build). Read both before deciding.

This document answers three questions in order: (1) where the time actually
goes today, (2) whether an existing tool — including OpenTofu/Terraform, a
tuned Ansible, or any of the faster Ansible-likes — already solves the
problem, and (3) what to build if not, and why that design.

The headline requirement it optimizes for (from [VISION.md](VISION.md)):
**a converged server must answer "0 changed, everything verified" in seconds,
and a drifted server must get exactly the drifted tasks re-applied.**

---

## 1. Where the ~15 minutes go today

For the common invocation — `--limit <one host>`, server already converged —
the cost decomposes into four structural layers. None of them is "the
playbooks are badly written"; all four are properties of Ansible's engine.

### 1.1 Per-task transport + interpreter tax (the biggest layer)

For **every task**, stock Ansible: builds an AnsiballZ zip payload (module +
recursively scanned `module_utils`), opens an SSH session (multiplexed, but
still a session), creates a remote temp dir, SFTPs the payload, **starts a
fresh Python interpreter on the target**, imports the module machinery,
executes, returns JSON, cleans up. This is 1–3 s per task even when the task
changes nothing. `setup-postgresql-nova.yml` has 65 tasks; the whole
repository has 452. At ~1.5–2.5 s of pure overhead per task, a 65-task no-op
run pays **2–4 minutes for zero work** — before any module logic runs.

Mitogen's measurements quantify this layer precisely: replacing only the
transport/interpreter layer (persistent remote interpreter, no re-upload)
yields 1.25–7x overall, 63x less bandwidth on a trivial loop, and cuts
network roundtrips for a file transfer from 57 to ~4
(<https://github.com/mitogen-hq/mitogen/blob/master/docs/ansible_detailed.rst>).

### 1.2 The 1Password lookup multiplier (the hidden layer)

`setup-postgresql-nova.yml` alone declares **13 `community.general.onepassword`
lookups** in play vars. Ansible evaluates lookups **lazily, on every template
reference** — a var used by five tasks spawns five `op` subprocesses, and
`postgres_readiness_users` (a list built from 8 lookup-vars, iterated by
loops over users/databases/privileges) re-renders on each loop iteration.
Each `op` call is ~0.3–2 s. Across the repo there are 40+ distinct lookups
but the *effective* subprocess count per run is far higher. This layer alone
plausibly accounts for **several minutes** of a nova/titan/delorean run, and
it is paid identically on no-op runs.

### 1.3 Module work that re-verifies from scratch every run

Even with a perfect transport, Ansible's idempotence model is "re-interrogate
the system every time": the `apt` module loads the apt cache per task (24 apt
tasks in the repo), each `postgresql_*` task opens its own DB connection
(44 such tasks on nova), `systemd` shells out per unit, `iptables` lists
rules per rule. Nothing remembers that the last run already verified this
exact state. This is the layer no existing tool fixes — and it is precisely
the "never repeat work that is already done" requirement.

### 1.4 Sequencing

With `--limit <one host>`, Ansible's host-level `forks` parallelism buys
nothing: 65 tasks run strictly sequentially, and fact gathering (~3–8 s)
runs first. Secret lookups are also sequential.

**Conclusion:** the pain is structural. Layers 1.1 and 1.2 can be reduced
inside the Ansible ecosystem; layer 1.3 cannot, and 1.3 is what stands
between "2 minutes" and "5 seconds".

---

## 2. Do existing tools solve this?

### 2.1 Tuning Ansible itself (the honest baseline)

`pipelining=true` (their sudoers permits it — root login), `gathering=smart`
with a fact cache, and freezing every lookup into `set_fact` once at play
start would plausibly take 15 min → **3–5 min**. Public data agrees:
pipelining+facts+forks typically cut 50–70%; and a well-tuned idempotent
playbook still took ~3 minutes in the best documented comparison
(<https://blog.hartwork.org/posts/replacing-ansible-with-salt-ssh-for-speed-and-for-good/>).
**Mitogen is not an option here**: it supports ansible-core 2.10–2.14 and the
controller runs core 2.21. Note also the controller currently runs with
`config file = None` — fully untuned defaults (see
[SKEPTIC.md](SKEPTIC.md) §2). **Verdict: a legitimate
interim relief, available this week, with no migration — but it plateaus at
minutes, not seconds, because layer 1.3 remains.**

### 2.2 OpenTofu / Terraform (the operator's specific question)

Tofu's model is: providers expose CRUD resources, a **state file** records
what was created, `plan` diffs desired config against state (+ provider
refresh). Applied to in-OS configuration this fails on exactly the
requirements that matter here:

- **The state file is trusted, reality is not probed.** A manual change on
  the server (the "dirty change" scenario) is invisible unless a provider
  implements refresh-by-reading-the-OS — and no maintained provider family
  reads packages/units/files/LVM over SSH. You would have to write that
  provider, which means building ruxel's hardest part anyway, then living
  inside Terraform's gRPC provider protocol, HCL, and resource-graph
  semantics.
- **Migration cost:** all 16 playbooks rewritten to HCL — the opposite of
  the drop-in requirement.
- **Imperative sequences** (drive init, Sentry's pause-for-manual-bootstrap,
  36 container restarts) map terribly to CRUD resources.

Where Tofu *is* right: the layer **above** ruxel — ordering Hetzner servers,
DNS, cloud firewalls — and for provisioning ruxel's own disposable test VMs.
**Verdict: borrow Tofu's plan/apply/diff UX and its "show me the diff before
touching anything" discipline; do not build on its runtime. For in-OS state,
reality-probing convergence (config management) is the correct model, not
state-file CRUD.**

### 2.3 The faster Ansible-likes

| Tool | Why it's fast | Why it doesn't close the gap |
|---|---|---|
| **pyinfra** (Python, ~4–6x) | Never ships code to targets — compiles state diffs to plain shell over persistent SSH; facts batched | Full rewrite to its Python API; no convergence cache — no-op still re-reads all facts every run; still Python startup costs |
| **Mitogen** (plugin) | Persistent remote interpreter, no re-upload | Still Ansible above it: lazy lookups, per-task module logic, layer 1.3 |
| **salt-ssh** | Ships whole runtime once per run | Heavy tarball model; measured ≈ parity with well-written Ansible |
| **Puppet Bolt** | — | Ruby startup + per-task ship; explicitly not for continuous state |
| **rash** (Rust) | Native modules, MiniJinja — proves the Rust YAML+Jinja engine works | **Local-only**: no SSH, no inventory, no remote anything; GPL-3.0 (concepts only, no code reuse) |
| **glidesh** (Rust, active 2026) | Native async SSH pool, two-phase check/apply | KDL language (full rewrite), stateless by design (no ledger), young |
| **jetporch** (Rust, dead 2023) | Rust engine, rayon over libssh2 | Discontinued; died on ecosystem economics ("every module needs a core contributor"), not on technology |

Two lessons transfer directly. From pyinfra/Mitogen: **the per-task
transport tax is the first thing to kill, and persistent connections plus
"ship nothing (or ship once)" kills it.** From jetporch's death: the moat
that kills general Ansible replacements is the *module ecosystem* — a
problem ruxel does not have, because [WORKLOAD.md](WORKLOAD.md) closes the
spec at **29 modules**. A closed-scope engine is a few engineer-months, not
a community project.

**Conclusion: nothing on the market combines (a) drop-in execution of these
exact files, (b) native remote execution with no interpreter, and (c) a
convergence cache that makes no-op runs near-instant. (c) exists nowhere at
all. Build ruxel.**

---

## 3. Recommended architecture

```
laptop (controller)                         each target host
┌─────────────────────────────┐            ┌──────────────────────────────┐
│ ruxel CLI                   │            │ ruxel-agent (static musl bin)│
│ 1 parse YAML (exact files)  │  one       │ uploaded once per version,   │
│ 2 resolve secrets (op, ‖)   │  mux'd     │ content-addressed            │
│ 3 render templates (minijinja) ── SSH ──▶│ 4 probe ledger (‖, <0.5 s)   │
│ 7 print plan / stream diff  │  channel   │ 5 verify mismatches (native) │
│                             │ ◀──────────│ 6 apply only the diff        │
└─────────────────────────────┘            │   ledger: /var/lib/ruxel     │
                                           └──────────────────────────────┘
```

### 3.1 Controller (runs on the operator's machine)

1. **Parse the existing files unchanged** — playbooks (`serde_norway`),
   `hosts.ini`, Jinja2 via **MiniJinja** (rash proves the pairing; add only
   the filters/behaviors in WORKLOAD.md). `ansible_python_interpreter` is
   parsed and ignored.
2. **Resolve secrets once, in parallel.** Deduplicate all `onepassword` /
   `pipe` lookups, resolve each distinct lookup exactly once per run through
   a reused `op` session, concurrently: 40 lookups ≈ 2–3 s total instead of
   minutes. Secrets live only in controller memory, are redacted in output
   (`no_log` honored), and never enter the ledger in recoverable form.
3. **Compile to a typed plan**: tasks fully rendered (loops expanded,
   conditionals attached, handlers wired), then streamed to the agent over
   one SSH channel. Playbook order is preserved within a host — Ansible
   semantics are sequential and the workload's shell tasks make aggressive
   auto-parallelization a correctness trap. Hosts run in parallel;
   **verification probes run massively in parallel** (read-only, safe);
   intra-host parallel *apply* is a later, opt-in optimization.

### 3.2 Transport

One **multiplexed SSH connection per host for the entire run**. Start with
the `openssh` crate (`native-mux` + `openssh-sftp-client`): it wraps the
operator's real OpenSSH setup, so keys/agent/`~/.ssh/config` behave
identically to today, and ControlMaster gives pooling for free. Keep the
transport behind a trait so pure-Rust `russh` can replace it if the extra
control is ever worth it. Agent delivery: SFTP once to
`/var/lib/ruxel/agent/<blake3>/ruxel-agent` (~5–10 MB, x86_64-musl — all six
hosts are x86_64), reused until the version hash changes.

### 3.3 Agent: native modules, no interpreter, batch-aware

The agent executes the 29-module set natively, and — unlike Ansible —
amortizes system interrogation across the whole plan:

- `file/copy/template/lineinfile/replace/blockinfile/stat/slurp` →
  std fs + blake3 hashing; idempotent edits computed in-process.
- `apt` → parse `/var/lib/dpkg/status` **once** for every package question
  in the plan; invoke `apt-get` only for actual changes.
- `systemd/service` → zbus (D-Bus) batch queries; at most one
  `daemon-reload` per run.
- `postgresql_db/user/privs/schema` → **one** `tokio-postgres` connection
  (local socket, `become_user: postgres` semantics) reused by all 44 tasks.
- `lvg/lvol/filesystem/mount` → lvm2/blkid shell-outs (same as Ansible's
  modules) + one parse of `/proc/mounts`, `blkid`, `vgs/lvs`.
- `iptables` → one `iptables-save` parse, batch diff of all 56 rules.
- `shell/command` → direct exec; `become_user` via setuid (agent runs as
  root, as the playbooks do); `eval "$(mise activate bash)"` patterns run
  as-is.
- `user/group/get_url/git/sysctl/authorized_key/timezone/assert/pause/debug`
  → straightforward native implementations; `pause` keeps the interactive
  Sentry bootstrap flow working.
- Facts: the agent reports the **3 fact paths the workload uses** in
  milliseconds; no general fact system.

### 3.4 The convergence ledger (the headline feature)

Per-host store at `/var/lib/ruxel/ledger` (single-writer, e.g. `redb` or
append-only JSONL). After a task verifies or applies successfully, the agent
records:

- **task identity**: stable hash of (playbook, task name, module, rendered
  params — secrets salted-hashed, never recoverable);
- **observed-state fingerprints**: the cheap probes that characterize the
  state this task guarantees — file (blake3, size, mtime), package
  (name=version from dpkg), unit (enabled/active + unit-file hash), mount
  (source UUID + mountpoint), sysctl value, VG/LV presence, DB object
  existence, marker files.

On the next run: the agent receives the plan, evaluates **all fingerprints
concurrently** (hundreds of stats/reads ≈ <0.5 s), and:

- all probes match **and** task identity unchanged → `verified (cached)` —
  microseconds, no module logic;
- any probe mismatch (= drift, the "dirty manual change") or changed params
  → full native module verification → apply only if genuinely needed;
- ledger missing/corrupt/older agent version → graceful degradation to full
  native verification (still minutes faster than Ansible);
- `--no-cache` forces full verification everywhere (paranoid mode).

The fast path is **never trusted blindly**: mtime+size alone never confirms
a file (hash on any doubt), and every mismatch falls through to the real
check. The ledger makes no-op cheap; correctness always has the last word.

### 3.5 CLI surface

```bash
ruxel plan  -i hosts.ini --limit postgresql-nova setup-postgresql-nova.yml   # trustworthy --check --diff, in seconds
ruxel apply -i hosts.ini --limit postgresql-nova setup-postgresql-nova.yml  # applies exactly the diff
ruxel apply -i hosts.ini install-base.yml                                    # all hosts, parallel
ruxel apply ... --tags velnor                                                # parses & works (sentry), but reruns are cheap enough not to need it
ruxel apply ... --no-cache                                                   # ledger bypass
```

### 3.6 Optional daemon tier (deferred)

The same agent binary can later run as `ruxel-agent --daemon`: inotify on
ledger-managed paths + periodic probes, answering "verified" from memory in
milliseconds and reporting drift proactively. Deferred because the ephemeral
agent already meets the <5 s target, and a daemon adds lifecycle, upgrade,
and security surface. The architecture leaves the door open (same binary,
same ledger, same protocol over a local socket).

### 3.7 Performance budget (converged `setup-postgresql-nova.yml`)

| Step | Cost |
|---|---|
| SSH mux connect + agent handshake | ~0.3–0.5 s |
| Parse + compile (controller, local) | ~50 ms |
| Secret resolution (parallel, deduplicated) | ~2–3 s (first run of the day; cheaper with `op` session reuse) |
| Plan stream + parallel ledger probes (65 tasks) | ~0.5 s |
| Result stream + diff render | ~0.1 s |
| **Total** | **≈ 3–5 s** (vs ~15 min today) |

First-run provisioning: dominated by real work (apt downloads, mkfs, initdb);
executor overhead drops from minutes to seconds, and hosts parallelize.

---

## 4. Risks and open questions

1. **Jinja2 fidelity.** MiniJinja covers the core; the workload's filter use
   is modest (`default()`, lookups). Mitigation: an offline corpus test —
   render all 22 templates + every templated task param with dummy secrets
   and diff against Ansible's rendering, entirely locally.
2. **Module semantic parity** — `postgresql_privs` idempotence and `apt`
   state edge cases are the subtle ones. Mitigation: side-by-side end-state
   diffing on disposable VMs (Phase 4), module by module.
3. **Ledger trust boundary.** Fingerprints must capture *all* state a task
   guarantees, or drift hides. Rule: when in doubt, probe more or fall
   through to full verification; `--no-cache` always exists.
4. **`op` behavior** under 40 parallel reads (rate limits, biometric session
   prompts) — needs a controlled experiment, controller-side only.
5. **Loop/register/when corner semantics** (e.g. `results` list shape) —
   covered by the closed spec; build a golden-output test per pattern in
   WORKLOAD.md.
6. **Scope creep.** The spec is closed at 29 modules; any new playbook
   feature lands in WORKLOAD.md first, code second.

## 5. Phased plan (no server contact at any phase)

- **Phase 1 — Fidelity, offline.** Parser + renderer + plan compiler. All 16
  playbooks parse; all templates render identically to Ansible (dummy
  secrets); golden tests for loops/when/register shapes. Pure local work.
- **Phase 2 — Agent core + ledger.** 8 core modules (file, copy, template,
  stat, lineinfile, apt, systemd, shell), protocol, ledger, `plan`/`apply`
  against **local Debian 12 VMs/containers** replicating `install-base.yml`.
  First benchmark numbers.
- **Phase 3 — Full module set.** postgresql (against a VM with PG 18), LVM
  (loop devices), iptables, the Sentry pause flow, tags, handlers,
  block/rescue.
- **Phase 4 — Acceptance.** Fresh VM: `ansible-playbook` vs `ruxel apply`,
  byte-compare resulting state (packages, files, units, mounts, DB objects);
  converged-rerun benchmark vs the <5 s goal. Only after this, an
  operator-supervised pilot on one production host, explicitly authorized
  per occasion ([AGENTS.md](../AGENTS.md) rule stands).
- **Parallel, optional, zero-risk relief now:** pipelining + smart facts +
  fact cache + `set_fact`-frozen lookups on the existing Ansible setup
  (Mitogen is incompatible with core 2.21) — buys 2–4x while ruxel is
  built. Independent of ruxel; operator's call. See the staged ladder in
  [SKEPTIC.md](SKEPTIC.md) §5, which supersedes the phase ordering above
  with a verify-first slice.

## 6. The recommendation in one paragraph

Keep the playbooks exactly as they are — they are a correct description of
the desired state. Replace the executor. Build ruxel as a Rust controller +
ephemeral static Rust agent over one multiplexed SSH connection, with all
secrets resolved once and in parallel on the controller, the 29-module
workload implemented natively and batch-aware on the agent, and a per-host
convergence ledger that turns "verify everything" into a sub-second parallel
fingerprint pass with full-verification fallback. Use OpenTofu, if at all,
for the layer above (and for test-VM fixtures) — not for in-OS state. This
is the only design on the table that makes the converged rerun cost seconds
while *strengthening*, not weakening, the guarantee that a full rerun proves
the server's state.

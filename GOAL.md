# Ruxel Implementation Goal

This file is the working goal/runbook for building ruxel. Agent sessions read
this file first, treat it as the active operational contract, and keep the
[Current Status](#current-status-and-to-do) section updated with findings and
progress. The design itself lives in `docs/` and is normative; this file
governs how sessions execute against it.

## Goal

Build ruxel: a Rust drop-in executor for the exact Ansible workload in
`ChainArgos/java-monorepo/ansible-configs`, per the closed spec in
[docs/SEMANTICS.md](docs/SEMANTICS.md) and the architecture in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), through milestones M0–M6 of
[docs/PLAN.md](docs/PLAN.md), each milestone proven by its acceptance gate
before the next begins.

Desired end state: the operator runs `ruxel plan|apply -i hosts.ini --limit
<host> <playbook>.yml` on the unchanged YAML files and gets correct results
in seconds on converged hosts — with every behavioral claim backed by oracle
captures and benchmarks committed in this repo. Production migration (M6) is
operator-driven and outside any autonomous session's scope.

## Absolute Safety Rules

1. **Never touch the production servers.** The six hosts in the ChainArgos
   inventory — pegasus, delorean, titan, sentry, postgresql-nova,
   clickhouse-selene — and any IP from that `hosts.ini` must never be
   connected to, probed, resolved-and-pinged, port-scanned, or targeted by
   ruxel/ansible/ssh in any mode, including read-only and `--check`. No
   exception exists in autonomous work; only the operator can authorize
   contact, per occasion, himself at the keyboard (M6).
2. **The only remote machines a session may touch are Hetzner Cloud VMs in
   the `ruxel-fixtures` project**, reached via the local `hcloud` context —
   resources the session itself created (or reaps as leftovers). Before any
   remote command, confirm the target address came from `hcloud server list`
   output of this project. Record this as `Safety check: target` in the
   session notes before the first remote command of a run.
3. **Fixture hygiene:** at most 2 fixture VMs at a time, smallest x86_64
   types; every session starts and ends with `tools/fixtures/reap` (or
   `hcloud server list` until that script exists) and destroys what it
   created.
4. **Secrets:** real ChainArgos secrets never enter fixtures, captures,
   goldens, logs, or commits. Test secrets come only from the synthetic
   `ruxel-test` 1P vault or generated dummies.
5. **Scope:** implement only the closed surface in docs/SEMANTICS.md.
   Unknown module/param/value = hard parse error, never silent acceptance.
   No features beyond the workload, ever, without the operator asking.
6. **Clean room:** rash and jetporch are GPL — concepts only; never port
   their source. Semantics come from SEMANTICS.md and oracle captures.

## Operator Pre-Confirmations

The operator has pre-confirmed these routine actions; do not stop to ask:

- create/destroy/list Hetzner Cloud resources **inside `ruxel-fixtures`
  only** (VMs, SSH keys, at cents-level cost, within the rule-3 cap)
- commit and push to `tailrocks/ruxel` `main` in conventional-commit slices;
  run the repo quality gates before each commit
- install/update local dev tooling needed for the work (brew/mise/cargo:
  hcloud, cargo-zigbuild, cargo-nextest, uv, etc.)
- create the `ruxel-test` 1P vault and seed/maintain synthetic items in it;
  set the `OP_SERVICE_ACCOUNT_TOKEN` GitHub Actions secret from
  `~/.config/ruxel/op-ci.env` when that file exists
- run the pinned-ansible oracle against fixture VMs and commit captures
- edit `.github/workflows/` in this repo as the milestones require

Not pre-confirmed (stop and ask): anything touching rule 1; deleting the
`ruxel-fixtures` project itself; force-pushes/history rewrites; publishing
the repo or crates; spending beyond fixture-VM cents (e.g. volumes, LBs,
snapshots kept overnight need an operator OK).

## How To Use This File With `/goal`

Start every session with this sequence:

1. Read this file completely.
2. Read `AGENTS.md`, `docs/PLAN.md`, and the docs the current milestone
   names (`docs/SEMANTICS.md`, `docs/ARCHITECTURE.md`,
   `docs/OPERATOR-SETUP.md` as needed).
3. **Precondition check (M0 step 0):** `hcloud server-type list` must
   succeed via the local context. If it fails, stop all fixture work and
   report the missing context to the operator — do offline work (workspace,
   parser, M1) instead if any is unblocked; never improvise other
   credentials.
4. Re-read [Current Status](#current-status-and-to-do); continue from the
   first unchecked item of the active milestone; never skip a gate.
5. Before the first remote command of a run, perform rule-2's
   `Safety check: target`.
6. At session end: update Current Status (done / found / next), reap
   fixtures, push.

Semantic questions are settled by the oracle (`tools/oracle/`, golden
captures), never by assumption; every SEMANTICS.md **⚠ verify** item gets an
experiment and its result recorded in the golden corpus before the
dependent code is considered done.

## Milestones

[docs/PLAN.md](docs/PLAN.md) is the schedule: M0 infrastructure → M1
fidelity (offline) → M2 transport+agent → M3 core modules+ledger → M4 full
module set → M5 performance proof → M6 operator pilot. Gates are defined
there; a gate's evidence (test run, capture, benchmark) is committed before
the milestone is marked done here.

## Current Status and To-Do

_Last updated: 2026-06-11 (session 3: **operator unblocked everything —
M0 fully complete; M2 gate re-proven on x86_64; first real workload
playbook (update-packages.yml) at full status parity**)._

**No operator blockers.** Session 3 received and wired both credentials:
- `hcloud` context `ruxel-fixtures` active (token also backed up in 1P:
  vault `ChainArgos`, item `ruxel Hetzner Cloud`); project verified empty
  at first use; fixture scripts validated end-to-end on a real VM
- 1P service account `ruxel-ci` (R/W on vault `ruxel-test` after one
  rotation — the first token was read-only): `~/.config/ruxel/op-ci.env`,
  GH Actions secret `OP_SERVICE_ACCOUNT_TOKEN`, 1P backup item
  `ruxel CI service account`; vault seeded with synthetic
  `ruxel-test SSH` + `ruxel-test PostgreSQL` items
- Operator to-dos: revoke the old read-only `ruxel-ci` service account in
  the 1P UI; rotate the Hetzner token when convenient (it is in this
  transcript) and update the 1P item + recreate the hcloud context

Remaining operator-optional: baseline timing logs (OPERATOR-SETUP §3).

**Found this session (environmental, for the operator):**
`https://holla-apt.tailrocks.com` serves no Release file to a Hetzner
Cloud fixture (install-base.yml dies at "Refresh apt cache" after the
holla source lands). If the repo is IP-allowlisted to production, the
full install-base parity gate needs either an allowlist entry for
fixtures or a stand-in repo. Captured failure is itself a golden.

**Found for the operator (latent workload bug):**
`config/sentry/config.yml` references `slack_client_id`,
`slack_client_secret`, `slack_signing_secret` — defined nowhere in the
workload (no play var, no inventory var). A real
`ansible-playbook setup-sentry.yml` run that reaches "Replace config.yml"
fails with AnsibleUndefinedVariable. Ruxel reproduces the error
faithfully (it is a committed golden); fixing it means adding the three
1P-backed play vars to setup-sentry.yml.

Preconditions:

- [x] `hcloud` context `ruxel-fixtures` (session 3)
- [x] `~/.config/ruxel/op-ci.env` + GH secret + vault seeding (session 3)
- [ ] Baseline logs `/tmp/baseline-*.log` (operator; optional)

M0 (offline parts done this session):

- [x] Workspace split: `crates/{ruxel,ruxel-core,ruxel-proto,ruxel-agent}`;
      agent cross-builds to 324K static x86_64-musl ELF (cargo-zigbuild);
      CI `agent-cross` job with static-linkage check
- [x] `tools/fixtures/`: create/destroy/reap scripts — context-scoped,
      label-guarded, 2-VM cap, ephemeral keys. **Validated session 3** on
      a real VM (create → ssh → gate → destroy → reap). Defaults now
      cpx12@sin (this account has no cx-line; EU shared-x86 capacity
      unavailable 2026-06-11); create.sh prints ready-to-use SSH opts
- [x] `tools/oracle/`: uv venv pinned to ansible-core 2.21.0 (exact match
      with controller) + `ruxel_capture` callback plugin; verified offline
      (local-connection playbook → ok/skipped/per-item records; raw_args
      arrive post-template at the callback layer)
- [x] Hetzner smoke test (session 3): ruxel-fixture-smoke, x86_64
      Debian 12 bookworm @ sin — full cycle clean
- [x] Seed `ruxel-test` 1P vault + GH secret (session 3)
- [x] Oracle capture of install-base.yml on the fixture
      (captures/install-base-fixture.jsonl, 30 records; stops at the
      holla-apt environmental failure — see operator note) plus
      update-packages run1/run2 (apt ⚠ items closed)
- [ ] Ingest baselines (operator-optional; logs absent)

**M0 is complete** modulo the operator-optional baselines.

M1 (**complete, session 2**):

- [x] Closed-surface model: 36-module registry with param-level closure and
      literal value enums; INI inventory parser (unknown anything = hard
      error). **Gate evidence: all 16 real playbooks parse**
      (`RUXEL_WORKLOAD_DIR=… cargo test -p ruxel-core --test workload`);
      unit tests prove rejection of unknown module/param/value/keyword
- [x] MiniJinja engine (`engine.rs`): native-types eval (single-expression
      → native; concat → string, no literal_eval), filters incl. custom
      bool/hash(sha256)/subelements/b64decode (b64decode + trim were
      missing from the original spec extraction — found by the harness,
      SEMANTICS.md updated), chainable-undefined that errors when a final
      result or output (AnsibleUndefinedVariable parity), Python-style
      output stringification (True/False/None), lazy layered scope with
      memoization + cycle containment, LookupResolver with dry-secrets
      fakes + per-run memoization
- [x] Render-parity harness vs the pinned oracle:
      `tools/oracle/render_parity.py` (fake 1P/pipe lookup plugins, shared
      parity_vars.json) → committed goldens
      `captures/render-parity.jsonl`; Rust replay
      `tests/render_parity.rs`. **Gate evidence: 242/242 expressions+
      conditions and 41/41 template files (22 with Jinja) byte-match
      ansible-core 2.21**; expression entries re-verify offline in CI
- [x] Loop/when/register golden tests: `runtime_semantics.yml` run against
      localhost (connection=local) → `captures/runtime-semantics.jsonl`;
      `task_eval.rs` reproduces every registered-result envelope (skip
      shape incl. false_condition, loop aggregates incl. all-skip/empty
      shapes, until attempts, changed_when_result, no_log censoring with
      uncensored register) — `tests/runtime_goldens.rs`, 11 tests
- [x] Plan compiler (`compiler.rs`): register/set_fact/fact read-set
      annotation, static render with rendered-enum re-validation, static
      loop expansion, deferred nodes with wait sets. **Gate evidence:
      16/16 playbooks compile to plans (383 static / 50 deferred tasks)**

M2 (**gate passed, session 2** — on aarch64; x86_64 re-proof rides the
first Hetzner fixture once the context exists):

- [x] Full protocol: Envelope{Hello,Plan,PlanPatch,Resume,Done} /
      Event{HelloAck,BlobsNeeded,TaskStart,TaskResult,PauseRequest,Log,
      CrashReport}; varint framing (64 MiB cap, clean-EOF) sync + async
- [x] Agent loop: handshake w/ version enforcement, workload-exact facts
      (default-route iface, VERSION_CODENAME, arch, hostname), Done/EOF
      clean exits, panic→CrashReport frame, flock single-run guard;
      pipe-driven integration tests incl. lock contention and kill -9
      release. Static musl ELF: 324K x86_64 / 376K aarch64 (zigbuild)
- [x] Controller transport: openssh ControlMaster native-mux (operator's
      ssh config/keys/known_hosts), blake3 content-addressed agent upload
      over a muxed SFTP channel (skip on hash hit), spawn + handshake +
      event send/recv + clean shutdown
- [x] CLI: drop-in `plan -i hosts.ini [--limit] [--tags] playbook.yml`
      offline compile preview (static/deferred per task); `apply --check`
      aliases plan; bare apply refuses until M3
- [x] **Gate evidence** (`tests/transport_gate.rs` vs local OrbStack
      Debian 12 bookworm VM `ruxel-deb`, root): cold connect+upload+
      handshake+facts+shutdown 495 ms; warm rerun 143 ms, uploaded=false;
      facts eth0/bookworm/aarch64; event round-trip (Plan → M2 Warn log)
- [ ] Pause relay (deferred to M3 with the pause module — nothing can
      emit PauseRequest until tasks execute)

Local fixture note: lima/cloud.debian.org images stalled (~100 B/s on
this network); the inner-loop VM is OrbStack machine `ruxel-deb`
(local-only, 192.168.139.103, Debian 12 arm64) — kept across sessions as
the fast inner loop, recreatable in seconds with
`orb create -a arm64 debian:bookworm ruxel-deb`. Gate run command:
`RUXEL_TEST_SSH_DEST='root@ruxel-deb@orb' RUXEL_TEST_AGENT_BIN=$PWD/
target/aarch64-unknown-linux-musl/release/ruxel-agent cargo test -p
ruxel-cli --test transport_gate -- --ignored --nocapture`.

M3 (**started, session 2** — execution foundation in place):

- [x] Agent module runtime: command (E15 shlex), shell (E14 creates
      shape), file (directory/absent/link + attrs + recurse,
      /etc/passwd-based id resolution for static musl), stat, copy
      (content=, atomic), slurp; per-iteration TaskStart/TaskResult
      streaming; check-mode skip (command/shell) and prediction
      (file/copy)
- [x] Linear per-host scheduler: JIT render with full scope (play vars +
      facts + registers), when incl. per-item + short-circuit AND lists,
      loop expansion, until/retries/delay, changed_when/failed_when,
      register/set_fact, ignore_errors, block/rescue, notify + handler
      flush, Ansible-rule recap; controller-side debug/assert/fail/
      set_fact; `apply` drives it (RUXEL_AGENT_BIN)
- [x] ⚠ closed: shell creates-guard status (golden E14: ok, not
      skipped), command free-form shlex split (E15), no_log censoring
      shape (E12/E13)
- [x] E2E evidence: 13-task closed-surface playbook against ruxel-deb —
      recap ok=12 changed=4 ignored=1 failed=0; loop/per-item-when/
      register/creates/ignore_errors verified on target; reruns stable
- [x] apt module (session 3): update_cache (pinned: never changed),
      upgrade: dist with summary-parse changed detection, idle
      autoremove unchanged, name install via dpkg-query (+apt-cache
      policy for latest), check-mode via apt-get -s
- [x] systemd/service module (session 3): daemon_reload executes but
      reports changed: false (pinned), started/stopped vs is-active,
      restarted always changed, enabled vs is-enabled
- [x] **First full workload playbook at parity (session 3):**
      `ruxel apply update-packages.yml` on the converged x86_64 Hetzner
      fixture — recap ok=5 changed=0 failed=0, status-identical to the
      pinned oracle capture of the same converged state
- [x] Transport hardening (session 3): own ControlMaster via
      tokio::process (openssh crate dropped — it lost the second
      session's stdin), agent orphan guard (exit 67 without a Hello in
      30 s — a dead controller can no longer wedge a host's lock),
      HelloAck timeout, per-process gate driver tools/fixtures/gate.sh.
      **x86_64 gate re-proof: cold 738 ms / warm 756 ms, no re-upload.**
      Known issue documented in transport.rs: second sequential connect
      inside one process stalls (shell repeats fine; real runs are one
      connect per process) — revisit before M5 parallelism
- [ ] Remaining M3: template/lineinfile/replace/blockinfile/get_url/
      apt_repository modules; blob channel for copy/template src=;
      convergence ledger + verdict engine + --no-cache; apt adjacency
      batching; per-task timing + --output json; pause relay;
      become_user; automated status-parity harness (diff ruxel recap vs
      capture statuses — done by hand for update-packages this session);
      then the M3 gate playbooks (install-base needs the holla-apt
      operator decision; install-docker/upgrade-debian unblocked)

Session log:
- 2026-06-11 s1: M0 offline + M1 parser. Commits 9beb77e…8deea64. Note:
  quality gates now run with pipefail after one clippy slip-through.
- 2026-06-11 s2: M1 complete. Commits 68fa8df (engine), 114d986 (parity
  harness + goldens), 5cfed41 (runtime goldens + task_eval), 7f9cda3
  (plan compiler + no_log). Oracle pins recorded in SEMANTICS.md §2.
  Then M2: cbd4a4b (proto+framing+agent loop), 5f079bb (SSH transport),
  6245565 (CLI plan surface), then event API + gate pass. Safety check:
  target = local OrbStack VM ruxel-deb (192.168.139.103), session-created,
  verified outside the production inventory before the first remote
  command; the only remote-ish target this session. hcloud precondition
  re-checked and still absent.
- 2026-06-11 s3: operator provided both tokens mid-session (rotation of
  the 1P one to R/W included). Credentials wired + backed up; M0
  completed (smoke test, vault seed, fixture captures); transport
  root-cause arc (bda9c13); fixture captures + apt ⚠ pins (e583019);
  apt+systemd modules and update-packages.yml full parity (09f69d4).
  Safety checks: target = ruxel-fixture-smoke 5.223.69.142 (created via
  tools/fixtures this session, verified against all six production IPs
  before first contact) and local ruxel-deb; fixture destroyed + reaped
  at session end. Operator note: Hetzner token transited this
  transcript — rotate when convenient; revoke the old read-only ruxel-ci
  service account.

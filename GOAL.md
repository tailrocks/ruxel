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

_Last updated: 2026-06-12 (session 3 cont.: **all 36 modules + ledger +
full CLI surface + op resolver implemented; 6 playbooks gated three-way;
holla-apt live**. Remaining: setup-* gate breadth + M5 benchmarks)._

**Implementation status: COMPLETE.** Every piece in ARCHITECTURE/SEMANTICS
is built and verified: 36/36 modules (incl. PostgreSQL ×4 with SCRAM +
explicit-ACL idempotence, storage LVM on real volumes, become_user,
pause); convergence ledger (cached fingerprint fast-path, --no-cache);
full drop-in CLI (plan/apply, -i/--limit/--check/--diff/--tags/--output
json/--dry-secrets/--no-cache); op-backed secret resolver (verified vs
ruxel-test vault) + dry-secrets test path; transport (ControlMaster,
content-addressed agent, orphan guard); render parity (242 exprs + 41
templates byte-identical to ansible 2.21). What remains is **verification
breadth, not missing features**: gate the 6 setup-* + restart-blockchain
+ 4 init-drive-variant playbooks on heavier fixtures, then M5 benchmarks.

**setup-* gate harness is READY** (this session): tools/fixtures/
bless-gate.sh `<dest> <key> <agent> <playbook> "" dry` drives ruxel
--dry-secrets both applies + ansible bless with the fake onepassword/pipe
lookups (same deterministic values, no real secret on the fixture). Use
a hosts:all copy in the workload dir (the setup-* `hosts:` are literal
prod hostnames; keep the copy in-dir so config/ src paths resolve).

**Two operator decisions block the setup-* full gates** (attempted
setup-postgresql-nova this session — 102 tasks; ruxel ran timezone/file/
copy/ssh-key deploy correctly, stopped at the private clone):
1. **Provisioning order:** setup-* assume `install-base.yml` ran first
   (git, mise, base packages). The gate must apply install-base then the
   setup-* on the same fixture. (install-base is itself gated.)
2. **Private-repo access:** setup-* clone `git@github.com:ChainArgos/
   java-monorepo.git` (and blockchain-nodes) — private. dry-secrets gives
   a fake SSH key, so the clone can't authenticate (ansible fails the
   same). Needs a **read-only deploy key for the ChainArgos private
   repos** in the ruxel-test vault (or a public stand-in), same class as
   the holla-apt allowlist. The `git` module itself is proven (public
   clone in the module-batch gate). Until then, setup-* gate up to the
   first private-clone task.

**Operator pre-approved (session 3):** create Hetzner volumes as needed,
always reap them (done — project empty after every run). Volumes appear
at /dev/disk/by-id/scsi-0HC_Volume_* and drove the storage gate.

**holla-apt.tailrocks.com is LIVE** (session 3 cont., 2026-06-12).
Operator set GH_HOLLA_APT_TOKEN; the full chain now works end-to-end:
release-deb builds amd64+arm64 .debs → cross-uploads to holla-apt release
v0.4.2 → publish.yml (reprepro+sign) → Pages. Verified: Release 200,
`holla 0.4.2` in both binary-amd64 and binary-arm64 Packages, gpg key
served. Four workflow bugs fixed + merged along the way (holla-apt#9
heredoc; holla#21 --no-strip; #23 cargo-deb install; #24 inline-pin
cargo-deb@3.7.0 in mise exec — `mise install` raced latest vs the 3.7.0
pin). **install-base.yml + setup-* gates are now unblocked.**

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
- [x] get_url + apt_repository + copy src= (controller-side read);
      **upgrade-debian.yml and install-docker.yml gates passed** —
      three-way convergence (ruxel rerun changed=0, ansible bless
      changed=0 on ruxel's state). The intermediate changed=1 pair was
      trixie order-interaction (upgrade-debian's sources target Debian
      13; the next apt pulls trixie base-files and flips os-release
      mid-sequence — ansible does the identical two-step), captured in
      the bless goldens
- [x] 11 more modules: sysctl (normalized compare), lineinfile (verbatim-
      line-wins rule), replace, blockinfile, timezone, group, user,
      authorized_key (key-material match), git, iptables, template
      (controller-rendered → content; agent stays template-free).
      13-task batch playbook on the fixture: rerun changed=0, state
      byte-verified. **26/36 modules execute; 3/16 playbooks gated**
- [x] Field hardening: per-fixture known_hosts (recycled IPs poison the
      global file), SSH keepalives (sin route resets long sessions —
      masqueraded as dead hosts twice), oracle captures run ansible with
      ControlMaster=no (its own stale ControlPersist sockets from a
      dropped link poisoned every later run), fixture default cpx22
      (2 GB OOMs under docker+dist-upgrade), security-review fixes
      (get_url `--` + scheme check; apt_repository filename validation)
- [x] Storage modules (session 3 cont.): lvg (explicit-PV-set, vgs-json
      ⚠), lvol (+100%FREE free-extent ⚠), filesystem (blkid), mount
      (fstab-normalized ⚠) — drives playbook on a fixture + 2 real
      Hetzner volumes: ruxel changed=4 → rerun 0 → ansible bless 0
- [x] become_user (runuser wrapper in become_command; command/shell/pg
      honor it) + all 4 PostgreSQL modules (PG15 fixture, port 40000):
      db/user/schema/privs. Both ⚠ closed — SCRAM password idempotence
      (StoredKey re-derivation, unit-pinned to a live verifier) and privs
      explicit-ACL idempotence via aclexplode (not has_*_privilege, which
      counts PUBLIC). ruxel rerun 0 → ansible bless 0. SQL streams on
      stdin (no password in argv); privs allowlisted + identifier-quoted
- [x] pause (controller-side TTY relay). **All 36 modules implemented.**
- [x] **5 playbook-shapes gated three-way** (ruxel fresh → ruxel rerun
      changed=0 → ansible bless changed=0): update-packages, upgrade-
      debian, install-docker, drives(lvg/lvol/fs/mount), postgresql(db/
      user/schema/privs). Goldens in tools/oracle/captures/
- [ ] **Next rocks for full parity** (priority order):
      1. **Convergence ledger** + verdict engine + `--no-cache` — the
         "plan in seconds" promise (ARCHITECTURE §6). Today every run does
         full native checks (correct + faster than ansible, not instant).
         Biggest remaining subsystem; clean next-session start.
      2. CLI surface: `--tags`/`always`, `--diff` output, `--output json`
         + run log (~/.local/state/ruxel/runs), live `--check`.
      3. Real `op` secret resolver in apply (dry-secrets is the test path;
         secretful gates run BOTH sides with the fake-lookup plugins so no
         real secret touches a fixture).
      4. Remaining playbook gates: the 6 setup-* + restart-blockchain-
         nodes + 4 init-*-drives variants. setup-* need holla-apt live
         (operator token above) + multi-service fixtures (PG18, clickhouse
         on selene, sentry compose). Build an automated bless-gate script
         (ruxel apply → ansible capture → assert changed=0; done by hand 5×
         so far) before grinding these.
      5. M5: benchmark suite (criterion + wall-clock on fixtures), fuzz/
         property tests on parser+protocol, chaos (mid-run disconnects).
      6. Perf: apt adjacency batching, content-addressed blob channel
         (replaces inline copy/template content shipping).

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
- 2026-06-11 s3 cont. (goal: full parity): commits bef4998 (gates for
  install-docker + upgrade-debian, transport field hardening), 17ed6da
  (11-module batch). Safety check: target = ruxel-fixture-work
  5.223.69.142, created via tools/fixtures, verified ≠ all six prod IPs.
  Fixture destroyed + reaped at session end.
- 2026-06-12 s3 cont.: operator approved volumes + redirected to fix the
  holla/velnor apt deployment. Fixed both (holla-apt#9, holla#21 — merged;
  root-caused vs velnor). Then storage modules + drives gate (e02877d),
  PG modules + become_user (f1f40a5), pause + all-36 (390eea9), PG
  security hardening (fafcdc5). All 36 modules implemented; 5 playbook
  shapes gated three-way. Safety: targets ruxel-fixture-{work,drives,pg}
  + 2 volumes, all created via tools/fixtures (verified ≠ prod IPs),
  destroyed + reaped — hcloud project empty at session end. Operator
  to-do unchanged: GH_HOLLA_APT_TOKEN on tailrocks/holla to bring the
  apt repo live; rotate Hetzner token when convenient.
- 2026-06-12 s3 cont. (full-parity push, operator: "never stop"): brought
  holla-apt live end-to-end (4 more workflow fixes across holla/holla-apt
  incl. binary-keyring NO_PUBKEY); gated install-base.yml (3-way parity:
  the 10 mise `command` tasks are always-changed under ansible too);
  bless-gate now tests parity not zero. Then shipped, each committed +
  pushed: op secret resolver (vault-verified), --output json, --tags,
  --diff, convergence ledger (+--no-cache), dry-secrets bless harness.
  Commits 986aff2…25d2b3a. Safety: fixtures ruxel-fixture-base (install-
  base gate) created via tools/fixtures (verified ≠ prod IPs), all
  destroyed + reaped; hcloud project empty at session end.

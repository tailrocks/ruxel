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

_Last updated: 2026-06-11 (session 2: **M1 complete — all gates green**;
starting M2)._

**Blocker for the operator:** the `hcloud` context `ruxel-fixtures` still
does not exist (`hcloud server-type list` fails; re-verified session 2).
All fixture-dependent work below is parked on it. OPERATOR-SETUP.md §1 —
~30 seconds in a separate terminal.

**Found for the operator (latent workload bug):**
`config/sentry/config.yml` references `slack_client_id`,
`slack_client_secret`, `slack_signing_secret` — defined nowhere in the
workload (no play var, no inventory var). A real
`ansible-playbook setup-sentry.yml` run that reaches "Replace config.yml"
fails with AnsibleUndefinedVariable. Ruxel reproduces the error
faithfully (it is a committed golden); fixing it means adding the three
1P-backed play vars to setup-sentry.yml.

Preconditions:

- [ ] `hcloud` context `ruxel-fixtures` (**operator** — blocks smoke test,
      fixture script validation, oracle VM captures)
- [ ] `~/.config/ruxel/op-ci.env` with 1P service-account token (operator;
      optional — blocks only CI secrets path)
- [ ] Baseline logs `/tmp/baseline-*.log` (operator; optional)

M0 (offline parts done this session):

- [x] Workspace split: `crates/{ruxel,ruxel-core,ruxel-proto,ruxel-agent}`;
      agent cross-builds to 324K static x86_64-musl ELF (cargo-zigbuild);
      CI `agent-cross` job with static-linkage check
- [x] `tools/fixtures/`: create/destroy/reap scripts written —
      context-scoped, label-guarded, 2-VM cap, ephemeral keys.
      **API-untested** (no context)
- [x] `tools/oracle/`: uv venv pinned to ansible-core 2.21.0 (exact match
      with controller) + `ruxel_capture` callback plugin; verified offline
      (local-connection playbook → ok/skipped/per-item records; raw_args
      arrive post-template at the callback layer)
- [ ] Hetzner smoke test (blocked: context)
- [ ] Seed `ruxel-test` 1P vault + GH secret (blocked: service account)
- [ ] Oracle capture of install-base.yml on a fixture VM (blocked: context)
- [ ] Ingest baselines (blocked: logs absent)

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

M2 (next): proto/ruxel.proto + prost codegen + framed stdio protocol;
openssh ControlMaster connection management; agent upload/handshake/facts;
SFTP blob channel. Gate needs a local Debian 12 VM (Lima/UTM — fixture
VMs still blocked on hcloud context; local VM is the PLAN-sanctioned
inner loop and touches no remote host).

Session log:
- 2026-06-11 s1: M0 offline + M1 parser. Commits 9beb77e…8deea64. Note:
  quality gates now run with pipefail after one clippy slip-through.
- 2026-06-11 s2: M1 complete. Commits 68fa8df (engine), 114d986 (parity
  harness + goldens), 5cfed41 (runtime goldens + task_eval), 7f9cda3
  (plan compiler + no_log). Oracle pins recorded in SEMANTICS.md §2.
  Safety: no remote commands this session (offline + localhost-only
  oracle runs); hcloud precondition re-checked and still absent.

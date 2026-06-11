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

_Last updated: 2026-06-11 (session: pre-implementation design)._

Done so far: design docs complete (SEMANTICS / ARCHITECTURE / PLAN /
WORKLOAD / VISION / DIRECTION / SKEPTIC / OPERATOR-SETUP); CLI scaffold with
plan/apply stubs; CI skeleton; spec extraction tooling decisions recorded.

Preconditions:

- [ ] `hcloud` context `ruxel-fixtures` exists and authenticates
      (**operator action** — OPERATOR-SETUP.md §1; blocks all fixture work)
- [ ] `~/.config/ruxel/op-ci.env` with 1P service-account token
      (operator action; optional now — blocks only the CI secrets path;
      flag, don't wait)
- [ ] Baseline logs `/tmp/baseline-*.log` (operator action; optional —
      ingest whenever they appear)

M0 to-do (in order, per PLAN.md):

- [ ] Hetzner smoke test: cheapest x86_64 Debian 12 VM in `ruxel-fixtures`,
      ephemeral SSH key, SSH in, `uname -m`, destroy VM+key, prove zero
      leftovers
- [ ] `tools/fixtures/`: create / destroy / list-and-reap scripts
- [ ] 1P CI path: create+seed `ruxel-test` vault (synthetic items mirroring
      WORKLOAD.md §2 lookup shapes); set GH secret if op-ci.env exists
- [ ] Workspace split: `crates/{ruxel,ruxel-agent,ruxel-proto,ruxel-core}`;
      agent x86_64-musl via cargo-zigbuild; CI builds all + cross
- [ ] `tools/oracle/`: uv-pinned ansible-core 2.21 + capture callback
      plugin; capture `install-base.yml` against a fixture VM (golden files)
- [ ] Ingest baselines into `docs/benchmarks/baseline/` if present
- [ ] Mark M0 gate evidence in this file; proceed to M1 without waiting

M1+ to-do: tracked here once M0 closes; definitions in PLAN.md.

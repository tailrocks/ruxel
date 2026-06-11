# PLAN — Build Order and Acceptance Gates

Status: committed plan, 2026-06-11. Operator decision: build the full
drop-in executor. Ordering below is **dependency order** (what must exist
before what can be proven), with explicit acceptance gates. Two standing
rules govern every milestone:

1. **Only the used surface** ([SEMANTICS.md](SEMANTICS.md) is normative;
   unknown module/param/value = hard error). No speculative features, no
   "while we're here" generality.
2. **No production contact, ever**, until M6's operator-driven pilot
   ([AGENTS.md](../AGENTS.md)). All gates run against local VMs/containers
   and the repo's own files.

Every **⚠ verify** item in SEMANTICS.md is an open question with an
experiment attached; a milestone is not done while one of its ⚠ items is
unresolved — they are resolved by *measurement against real Ansible*, not
by assumption.

---

## M1 — Fidelity layer (controller-side, fully offline)

Parser (inventory INI + playbook YAML → typed model), MiniJinja engine
with the workload's filter set (`default, bool, urlencode, map, list,
length, hash('sha256'), subelements` + native-types semantics), lookup
resolver with `--dry-secrets` mode (deterministic fakes), loop/when/
register/until evaluation, register-dependency DAG compiler, `no_log`
redaction.

**Parity harness** (the gate, and a permanent CI fixture): for every
playbook — all 16 — and every template — all 22 — render every expression
and template through ansible-core 2.21's Templar (driven via its Python
API locally) and through ruxel, with identical fake variable sets;
byte-diff. Includes the loop/when/register shapes extracted on 2026-06-11
(literal-list and template-string loops, list-AND `when`, per-item when,
registered-result attribute access).

**Gate:** 16/16 playbooks compile to plans; 22/22 templates byte-identical;
every inline expression identical; all M1-class ⚠ items closed with
recorded evidence.

## M2 — Transport + agent skeleton

`proto/ruxel.proto`, prost codegen, framed stdio protocol; `openssh`
ControlMaster connection management; agent upload (content-addressed) +
handshake + facts; SFTP blob channel; event stream plumbing; `pause`
relay; structured crash reporting; flock single-run guard.

**Gate:** against a local Debian 12 VM: cold connect → handshake → facts →
clean shutdown in < 1 s; agent re-upload only on hash change; kill -9 /
disconnect mid-stream leaves the target reusable (rerun succeeds).

## M3 — Core modules + ledger + plan/apply

Modules: `file, copy, template, lineinfile, replace, blockinfile, stat,
slurp, shell, command, apt, apt_repository, systemd, service, get_url,
debug, assert, fail, set_fact` — each with native check, apply, diff,
check-mode prediction, and ledger probe set (ARCHITECTURE §6 classes).
Ledger store + verdict engine + `--no-cache`. Handlers/notify. dpkg and
systemd snapshots; apt adjacency batching with per-task status
reconstruction. CLI: `plan`/`apply`, `-i`, `--limit`, `--check`, `--diff`,
ansible-shaped output + `--output json`.

**Gate (fixture VM, fresh Debian 12):** `install-base.yml`,
`install-docker.yml`, `upgrade-debian.yml`, `update-packages.yml` —
(a) ruxel apply from scratch produces end state byte-equivalent to
`ansible-playbook` from scratch on a twin VM (automated state-diff
harness: package set, file tree hashes under managed paths, unit states);
(b) converged `ruxel plan` < 2 s on the VM; (c) per-task status/changed
counts match Ansible's recap exactly on both first and second runs;
(d) M3 ⚠ items (apt update_cache/upgrade changed semantics, lineinfile
idempotence rule, daemon_reload changed, creates-guard status, become_user
env) closed by recorded side-by-side experiments.

## M4 — Full module set + full control flow

Remaining modules: `sysctl` (both spellings), `user, group,
authorized_key, iptables, git, pause, timezone, lvg, lvol, filesystem,
mount, postgresql_db, postgresql_user, postgresql_privs,
postgresql_schema`. Storage fixtures use loop devices for LVM/XFS/ext4;
PostgreSQL 18 + port 40000 fixture mirrors titan/nova; tags (`--tags`,
`always`), block/rescue, `until/retries/delay`, `environment`,
`become_user: postgres`, secret resolution against real `op` (read-only
items created for testing).

**Gate:** every one of the 16 playbooks converges on its fixture VM with
end-state equivalence vs Ansible (the M3 harness extended: LVM layout,
pg_catalog dump diff, iptables-save diff, crontab/unit diff); the
postgresql_user password idempotence and lvg/lvol ⚠ items closed;
`setup-sentry.yml`'s pause flow exercised end-to-end (fixture compose
standing in for Sentry's installer).

## M5 — Performance proof + hardening

Benchmark suite (criterion + wall-clock harness on VMs with simulated
RTT): converged no-op per playbook, edit-one-task rerun, fresh provision,
6-hosts-parallel run. Targets from ARCHITECTURE §10 — measured, recorded
in-repo, regressions gated in CI. Fuzz/property tests on parser and
protocol; chaos tests (mid-run disconnects at every protocol state).

**Gate:** converged `plan` ≤ 5 s including real 1P resolution against the
fixture fleet; no protocol state leaves a target unrecoverable; published
benchmark report in `docs/benchmarks/`.

## M6 — Operator pilot (production, operator-driven, plan-only first)

Sequence per host, each step individually authorized by the operator who
runs the commands himself: (1) `ruxel plan` read-only against one
low-risk host, output compared with `ansible-playbook --check --diff`;
(2) diffs reviewed; (3) first `ruxel apply` on a change the operator was
going to make anyway; (4) graduate host-by-host. Ansible remains installed
and authoritative until the operator retires it — the files never changed,
so there is nothing to migrate back.

## Standing workstreams (no milestone, always on)

- **Spec drift watch:** the param/value extractor from 2026-06-11 lives in
  `tools/spec-extract/` and runs in CI against the ansible-configs
  checkout; any new module/param/value appearing in the playbooks fails CI
  until SEMANTICS.md and the implementation cover it. The spec stays
  closed *and* current.
- **Warm-daemon tier** (ARCHITECTURE §9) and proactive drift reporting:
  designed, deliberately not scheduled until the ephemeral path is proven
  in M5 — it is an acceleration, not a dependency.

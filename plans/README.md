# Active implementation plan

Only unfinished work required for the ChainArgos-extracted closed surface is
listed here. Ansible-core 2.21 is the behavioral oracle. Real reference files
are offline extraction input only; every executable comparison uses committed
synthetic fixtures on provider-verified disposable targets. Production contact
is forbidden.

## 1. Finish executable fixture parity

- Regenerate deterministic captures that still contain machine/run-specific
  data using `normalize_capture.py`.
- Capture the remaining fixture playbooks: `control-flow`, `shape-variants`,
  `system-surface`, and `check-semantics` remote check/diff behavior.
- For every fixture, compare equivalent fresh Ansible and Ruxel targets, first
  apply, converged rerun, check/diff results, normalized per-task fields, and
  final state.
- Extend state snapshots for repository definitions, service enabled/active
  state, filesystem signatures, full Git state, PostgreSQL extensions/tables/
  ownership/ACL/default ACL, and every other observable used by the workload.
- Exercise storage and PostgreSQL fixtures on appropriately provisioned
  disposable targets; never substitute production configuration or devices.

Done when all 36 modules and all mapped interactions pass every run mode and
state comparison from committed synthetic sources.

## 2. Complete performance and resilience proof

- Benchmark Ansible and Ruxel on equivalent synthetic fresh, converged,
  one-task-drift, check/diff, secret, storage, PostgreSQL, simulated-RTT, and
  six-host cases.
- Store raw logs, exact versions, fixture specification, repetitions, summary
  statistics, and passing correctness results beside each report.
- Complete disconnect/kill chaos at every protocol state and prove the target
  is reusable afterward.
- Remove or relabel historical benchmark claims that used absent/private
  fixture sources.

Done when every required benchmark and chaos class is reproducible and remains
Ansible-equivalent.

## Already complete; not active work

- 675/675 extracted feature names exist in the fixture corpus.
- Synthetic render parity covers every fixture loop item, native/string
  rendering, templates, lookup behavior, and matching error classes.
- All semantic verification markers are measured and closed.
- Result comparison and server-state bless checks exist.
- Remote gates structurally accept only labeled fixture identities.
- Bounded six-way host orchestration passed a two-host real-fixture gate.
- All 675 extracted features have a CI-enforced fixture/oracle/Ruxel/state
  evidence chain.
- Six-host disposable-container concurrency, ordered recaps, unreachable
  output, and repeated transport-resource cleanup pass reproducibly.

Ruxel status remains: **core implemented; closed-workload compatibility proof
incomplete** until all four active sections pass.

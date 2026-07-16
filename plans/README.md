# Active implementation plan

Only unfinished work required for the ChainArgos-extracted closed surface is
listed here. Ansible-core 2.21 is the behavioral oracle. Real reference files
are offline extraction input only; every executable comparison uses committed
synthetic fixtures on provider-verified disposable targets. Production contact
is forbidden.

## 1. Complete evidence-chain traceability

- Extend the versioned extraction artifact so every required feature maps to
  its exact synthetic fixture task, normalized Ansible capture, Ruxel
  assertion, and observable-state assertion.
- Preserve distinct expression/register/control-flow input shapes instead of
  collapsing them to construct names.
- Fail CI when any evidence link is removed.

Done when every extracted shape has the reproducible chain:
`offline extraction -> fixture task -> Ansible oracle -> Ruxel assertion -> state assertion`.

## 2. Finish executable fixture parity

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

## 3. Finish multi-host acceptance

- Keep the historical transport-stall fix STOP-blocked unless a deterministic
  cause is reproduced. Bounded parallel host orchestration is implemented and
  passed the two-host disposable-fixture gate.
- Capture the remaining six-host disposable benchmark and repeated resource-
  leak/stall checks. Do not weaken the fixture safety cap silently; use six
  local disposable SSH targets or an operator-approved cap change.
- Complete unreachable-host output/run-log parity if the closed workload's
  operational acceptance requires it.

Done when six-host time is approximately the slowest host, ordered output and
recaps match the contract, and repeated runs leak no transport resources.

## 4. Complete performance and resilience proof

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

Ruxel status remains: **core implemented; closed-workload compatibility proof
incomplete** until all four active sections pass.

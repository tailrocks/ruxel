# Active implementation plan

No required implementation work remains for the ChainArgos-extracted closed
surface. Plans 001–025 are implemented and their acceptance evidence passes.

## Completion proof

- 675/675 extracted feature identities have a CI-enforced
  fixture/oracle/Ruxel/state evidence chain.
- All 14 synthetic fixtures pass current-binary fresh, converged, check/diff,
  normalized per-task result, and final-state parity.
- All 36 closed-surface modules and mapped interactions are executable.
- The complete ten-case correctness-coupled benchmark matrix passes, including
  storage, PostgreSQL, simulated RTT, six-host, and 65-task/52-lookup scale.
- The six-case deterministic SSH chaos matrix passes bounded recovery and
  process/socket/flock/temp-file leak checks.
- Provider identity is verified before remote execution. Benchmark manifests
  record disposable-resource reaping; the chaos owner always reaps through its
  exit trap after artifact creation.

Ansible-core 2.21 remains the behavioral oracle. Real ChainArgos files remain
offline extraction input only and production contact remains forbidden. Scope
does not include Ansible features absent from `docs/WORKLOAD.md`.

Newest stable dependencies and configuration are adopted deliberately, then
exact versions are pinned for reproducibility. Backward compatibility is not a
project requirement.

Evidence locations:

- `tools/oracle/captures/`
- `docs/benchmarks/results/`
- `tools/chaos/artifacts/`

Any future item belongs here only after the operator explicitly expands the
closed workload or requests a new project goal.

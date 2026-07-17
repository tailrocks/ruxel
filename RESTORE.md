# RESTORE — Current Handoff

Updated: 2026-07-17. Authoritative status: `GOAL.md` and `docs/PLAN.md`.

## Status

The implementation plan is complete for the closed ChainArgos-extracted
Ansible surface. Current binaries passed the full 14-fixture parity suite, the
ten-case correctness-coupled performance matrix, six-host and scale gates, and
all six deterministic SSH chaos boundaries. `plans/README.md` has no active
required items.

## Safety remains permanent

- Never contact, resolve, probe, or execute against production hosts.
- Real ChainArgos configuration is offline extraction input only.
- Never execute or render-compare real workload files, even on fixtures.
- Remote parity uses only repository synthetic fixtures and provider-verified
  disposable targets.
- Use synthetic repositories, secrets, identities, hostnames, and device data.
- Reap every disposable resource after a gate.

## Evidence

- Synthetic corpus: `tools/fixture-project/`
- Oracle captures and verifier: `tools/oracle/`
- Performance evidence: `docs/benchmarks/results/`
- Chaos evidence: `tools/chaos/artifacts/`
- Normative scope and behavior: `docs/WORKLOAD.md`, `docs/SEMANTICS.md`

Future work requires an explicit new operator goal. A production pilot is not
part of completion and is never inferred from this status.

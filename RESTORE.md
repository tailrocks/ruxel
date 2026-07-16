# RESTORE — Current Handoff

Updated: 2026-07-16. Authoritative backlog: `plans/README.md`.

## Status

Core execution is substantially implemented. Local Rust gates, exhaustive
synthetic render parity, semantic measurements, target safety, and two-host
parallel execution pass. Full compatibility proof still needs per-shape
evidence links, complete fresh/check/diff/final-state fixture gates, the
six-host benchmark, and performance/chaos coverage.

Never use the old “implementation complete” claim. Never count captures whose
source fixture is absent. Never use historical real-workload execution as
current reproducible acceptance evidence.

## Safety

- Never contact, resolve, probe, or execute against production hosts.
- Real ChainArgos configuration is offline extraction input only.
- Never execute or render-compare real workload files, even on fixtures.
- Remote parity uses only `tools/fixture-project/` and provider-verified
  disposable demo servers.
- Use synthetic repositories, secrets, identities, hostnames, and device data.
- Reap every disposable resource after a gate.

## Resume order

1. Finish per-shape fixture/oracle/Ruxel/state traceability.
2. Run every fixture through fresh, converged, check/diff, and final-state
   equivalence gates; extend snapshots where required.
3. Capture six-host disposable acceptance.
4. Run the complete correctness-coupled performance and chaos matrix.

## Useful locations

- Active plan: `plans/README.md`
- Synthetic corpus: `tools/fixture-project/`
- Oracle: `tools/oracle/`
- Fixture lifecycle: `tools/fixtures/`
- Normative semantics: `docs/SEMANTICS.md`

If remote credentials or infrastructure are unavailable, continue offline
corpus/oracle work. Report exact missing setup only when a remote acceptance
gate is ready.

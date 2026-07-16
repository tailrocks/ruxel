# RESTORE — Current Handoff

Updated: 2026-07-16. Authoritative backlog: `plans/README.md`.

## Status

Core execution is substantially implemented. Local Rust gates pass and seven
repository-owned synthetic fixtures exercise important storage, PostgreSQL,
delegation, restart, and snapshot paths. Full Ansible compatibility is not yet
proven: fixture breadth, render parity, end-state comparison, multi-host
execution, and performance/chaos coverage remain open.

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

1. Build extracted-shape-to-fixture traceability.
2. Restore the synthetic Ansible/Ruxel render oracle and close all semantic
   verification markers.
3. Expand the fixture project to every mapped module and interaction.
4. Compare normalized task results and final managed state.
5. Enforce fixture identity before remote execution.
6. Verify bounded multi-host execution on disposable demo servers.
7. Run the complete correctness-coupled performance and chaos matrix.

## Useful locations

- Active plan: `plans/README.md`
- Synthetic corpus: `tools/fixture-project/`
- Oracle: `tools/oracle/`
- Fixture lifecycle: `tools/fixtures/`
- Normative semantics: `docs/SEMANTICS.md`

If remote credentials or infrastructure are unavailable, continue offline
corpus/oracle work. Report exact missing setup only when a remote acceptance
gate is ready.

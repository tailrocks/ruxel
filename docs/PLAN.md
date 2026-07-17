# PLAN — Verified Ansible Compatibility

Status: **complete for the closed ChainArgos-extracted workload**.

Pinned Ansible 2.21 defines behavior only for the surface in
[`WORKLOAD.md`](WORKLOAD.md). Real reference configuration remains offline
feature-extraction input. Every executable comparison uses repository-owned
synthetic fixtures on provider-verified disposable targets. Production hosts
were not contacted and are never an automated acceptance target.

## Completed acceptance

- Every extracted feature shape is mapped through fixture, oracle, Ruxel, and
  observable-state evidence.
- All 14 fixtures agree on rendered inputs, fresh/converged/check-diff task
  outcomes, normalized result shapes, diffs, and final state.
- Storage and PostgreSQL cases run on purpose-provisioned disposable twins.
- Ten benchmark classes contain raw sanitized logs, hashes, exact versions,
  fixture specifications, three or more alternating repetitions, correctness
  results, summaries, and resource-reaping proof.
- Six-host execution and 65-task/52-lookup scale pass. Scale measured a
  1.785 s Ruxel median versus 64.912 s for Ansible (36.36x).
- Six deterministic protocol interruption cases reconnect, converge to equal
  state, release the agent flock, leak no processes/sockets/temp files, and
  recover within 120 seconds.
- Tooling rejects non-fixture remote identities before SSH.

Reproduce or verify evidence with the tools under `tools/oracle/`,
`tools/benchmarks/`, and `tools/chaos/`. Committed results live under
`tools/oracle/captures/`, `docs/benchmarks/results/`, and
`tools/chaos/artifacts/`.

This completion statement does not claim general Ansible compatibility and
does not authorize a production pilot. New workload shapes require explicit
operator scope expansion and new oracle evidence.

# Ruxel Implementation Goal

Build Ruxel as a Rust executor for exactly the Ansible surface closed by
`docs/WORKLOAD.md` and `docs/SEMANTICS.md`. Pinned Ansible 2.21 is the
behavioral oracle. No broader Ansible compatibility is a goal.

## Achieved end state

The unchanged supported synthetic configuration produces Ansible-equivalent
rendering, task outcomes, diffs, registered results, and managed-server state.
Committed disposable-target evidence also proves lower measured execution
time. The 65-task/52-lookup scale case measured a 1.785 s Ruxel median versus
64.912 s for Ansible (36.36x), with result and state parity.

## Absolute safety rules

1. Never connect to, resolve, probe, scan, or execute against the six production
   hosts or addresses from their inventory. This includes check mode.
2. Real ChainArgos configuration is read-only offline feature-extraction input.
   Never execute or render-compare it, even on fixtures.
3. Executable parity uses only committed synthetic sources under
   `tools/fixture-project/` and disposable local or Hetzner demo servers.
4. A remote target must be proven to belong to the isolated fixture project
   before any SSH, Ruxel, or Ansible process starts.
5. Real secrets, identities, hostnames, inventories, and device IDs never enter
   fixtures, captures, logs, or commits.
6. Implement only the extracted closed surface. Unknown modules, parameters,
   and values fail loudly.
7. Reap all disposable resources after each remote gate.

Only the operator can authorize production contact, separately and per host.
Production pilot work is outside this implementation goal.

## Working rules

- Semantic questions are settled by measured pinned-Ansible behavior.
- Dependency and configuration upgrades deliberately track newest stable
  releases, then pin exact resolved versions for reproducible evidence. No
  backward-compatibility surface is maintained.
- Each extracted feature shape maps to a synthetic fixture, Ansible capture,
  Ruxel assertion, and observable-state assertion.
- Correctness parity gates every performance claim.
- Commit and push completed changes on the current branch. Never create another
  branch.
- Before each commit run `just check`.

## Completion evidence

**The implementation plan is complete for the closed ChainArgos workload.**

- All 675 extracted feature identities have fixture/oracle/Ruxel/state evidence.
- All 14 executable fixtures pass fresh, converged, check/diff, normalized
  per-task result, and final-state parity using the current binaries.
- All 36 closed-surface modules and mapped control-flow interactions are covered.
- Ten correctness-coupled benchmark classes pass, including storage,
  PostgreSQL, simulated RTT, six-host, and 65-task scale.
- Six deterministic SSH interruption boundaries recover within 120 seconds
  with equal state and no flock, process, socket, or temporary-file leaks.
- Remote gates structurally accept only provider-verified disposable targets;
  benchmark manifests record resource reaping and the chaos owner always reaps
  through its exit trap.

Evidence is under `tools/oracle/captures/`, `docs/benchmarks/results/`, and
`tools/chaos/artifacts/`. `plans/README.md` records that no required active
implementation work remains.

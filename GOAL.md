# Ruxel Implementation Goal

Build Ruxel as a Rust executor for the exact Ansible surface defined by
`docs/WORKLOAD.md` and `docs/SEMANTICS.md`. Pinned Ansible 2.21 is the
behavioral oracle. The authoritative active backlog is `plans/README.md`.

## Desired end state

The operator can use the unchanged supported configuration with Ruxel and get
Ansible-equivalent rendering, task outcomes, diffs, registered results, and
managed server state, with lower measured execution time. Every claim must be
reproducible from committed synthetic fixtures and disposable demo servers.

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
Production pilot work is outside autonomous implementation.

## Working rules

- Semantic questions are settled by measured Ansible behavior, never memory or
  assumption.
- Each extracted feature shape maps to a synthetic fixture, Ansible capture,
  Ruxel assertion, and observable-state assertion.
- Aggregate recap counts are not parity proof; compare task identity/status,
  results, diffs, and final state.
- Correctness parity gates performance claims. Never optimize away required
  Ansible behavior.
- Commit and push completed changes on the current branch. Never create another
  branch.
- Before each commit run `cargo fmt --all --check`,
  `cargo clippy --all-targets -- -D warnings`, and `cargo nextest run`.

## Current status

**Core implemented; Ansible compatibility verification incomplete.** Current
unit tests and seven synthetic playbooks prove important slices, not the whole
closed workload. Historical render-parity and real-workload benchmark claims
are not current reproducible acceptance evidence.

Remaining required work:

1. finish fresh/converged/check/diff and final-state parity for every fixture;
2. complete correctness-coupled performance and chaos evidence.

Detailed acceptance criteria and execution order live in `plans/README.md`.
Do not describe Ruxel as complete until all seven workstreams pass.

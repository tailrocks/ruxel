# AGENTS.md

Rules for AI agents working in this repository.

## Hard rule: never touch the production servers

The reference workload for this project (`ChainArgos/java-monorepo/
ansible-configs`) targets six production servers. **No agent may ever
connect to, probe, port-scan, SSH into, run commands against, or otherwise
interact with those hosts — for any reason, in any mode, including
"read-only" checks.** This includes resolving their DNS, testing SSH
reachability, and running `ruxel`/`ansible` in check mode against them.

- Development, testing, and benchmarking happen exclusively against
  disposable targets (local VMs, containers, throwaway cloud hosts) that the
  operator provides explicitly per occasion.
- This rule has no exceptions and does not expire. Only the operator can
  authorize contact with a production host, individually, per occasion.
- If a task seems to require touching a production host, stop and ask.

## Project phase

Implementation. The execution engine is built and feature-complete (all
closed-surface modules, the convergence ledger, the full `plan`/`apply` CLI,
and the `op` secret resolver). Current work is verification breadth (gating
the remaining playbooks) and the M5 performance/hardening pass; M6 is the
operator-driven production pilot and stays untouched by autonomous work.

`GOAL.md` is the active operational contract and `RESTORE.md` the latest
state snapshot — read both at session start. The design docs in `docs/`
remain normative for behavior.

## Scope discipline

[docs/WORKLOAD.md](docs/WORKLOAD.md) is a closed spec. Do not add modules,
language features, or compatibility surface beyond it without the operator
asking. "Ansible has this feature" is not a reason to support something.

## Conventions

- Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`).
- Quality gates before a PR: `cargo fmt --all --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- Rust edition 2024, toolchain pinned in `rust-toolchain.toml`.

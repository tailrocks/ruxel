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

Research and design. Do not start building the execution engine until the
operator explicitly moves the project to implementation. The current
deliverables are the documents in `docs/` and the CLI skeleton.

## Scope discipline

[docs/WORKLOAD.md](docs/WORKLOAD.md) is a closed spec. Do not add modules,
language features, or compatibility surface beyond it without the operator
asking. "Ansible has this feature" is not a reason to support something.

## Conventions

- Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`).
- Quality gates before a PR: `cargo fmt --all --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- Rust edition 2024, toolchain pinned in `rust-toolchain.toml`.

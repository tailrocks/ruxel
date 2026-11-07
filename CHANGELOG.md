# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Ruxel.pkl schema v0.
- Engine secret wrappers, value redaction, and secret-aware ledger keys.
- JSON, YAML-compatible JSON, and TOML rendering for resolved configuration values.
- Typed Pkl configuration, evaluation via pklr, and `ruxel config dump`.
- Controller commands for plan, check, diff, and apply, with synthetic local acceptance coverage in progress.
- alint repository policy (tailrocks rust-strict-v1) in `mise run check` and CI.
- `hk.pkl` git hooks (mise, gitleaks, cargo fmt, check, clippy, deny, rumdl).
- Initial research implementation and closed-workload CLI/agent surface.

### Changed

- Agents finish the current iteration and flush the ledger on SIGTERM or SIGINT during a batch.
- Sensitive-resource tracking treats parent traversal and relative remote paths conservatively.
- Block check mode reaches nested tasks, rescue/always tasks and controller pauses; child overrides take precedence.
- Workspace lints from the operator's repository standard, clippy configuration.
- Crates moved to `crates/<group>/<package>` (repository policy).
- Dependencies updated to their newest versions.
- Toolchain: Rust 1.99.1, operator mise layout with mbx as the cargo wrapper, lockfile v4; uv removed.
- CI regenerated with velnor-actions 0.1.75; tool versions now come from `mise.toml`.

### Removed

- Duplicate legacy loop-result reason field; empty loops expose `skip_reason`.
- YAML and INI configuration input and MiniJinja authoring.
- Ansible parity tooling, Ansible-era docs, and agent modules no schema reaches; recoverable from tag `ansible-parity-final`.
- `justfile`; `mise run check` replaces `just check`. The Python Ansible-parity suites no longer run by default.

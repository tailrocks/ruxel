# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- A typed workflow language: workflows amend `modulepath:/ruxel/Workflow.pkl` and list stages of steps (`File`, `Packages`, `Service`, `Command`, `Capture`, …); recipes extend `modulepath:/ruxel/Ruxel.pkl`. `modulepath:/ruxel/internal/Lowering.pkl` lowers them to engine input and rejects broken references with plain messages.
- The binary serves its compiled-in schema as `modulepath:/ruxel/Ruxel.pkl`; `plan`, `check`, `diff` and `apply` refuse configuration evaluated against any other copy of the schema.
- `--module-path DIR[:DIR...]` on `config dump`, `plan`, `check`, `diff` and `apply` serves shared Pkl libraries, like `pkl eval --module-path`.
- `ruxel schema export DIR` writes the served schema for editors and the `pkl` CLI.
- Strict guard and SSH configuration types and retained evaluated input for future admission.
- Embed the canonical Pkl schema for configuration tooling.
- Engine secret wrappers, value redaction, and secret-aware ledger keys.
- JSON, YAML-compatible JSON, and TOML rendering for resolved configuration values.
- Typed Pkl configuration, evaluation via pklr, and `ruxel config dump`.
- Controller commands for plan, check, diff, and apply, with synthetic local acceptance coverage in progress.
- alint repository policy (tailrocks rust-strict-v1) in `mise run check` and CI.
- `hk.pkl` git hooks (mise, gitleaks, cargo fmt, check, clippy, deny, rumdl).
- Initial research implementation and closed-workload CLI/agent surface.

### Changed

- The engine schema moves to `modulepath:/ruxel/engine/Engine.pkl` (module `Engine`); `modulepath:/ruxel/Ruxel.pkl` is now the workflow language. Configuration that imports the engine schema must change its import. The schema fingerprint covers every served module, and `ruxel schema export` writes them all.
- Evaluation allows an expression depth of 256 instead of pklr's default of 32.
- Verify complete SSH argument vectors for guard admission modes.
- Downloads use trusted system clients with an isolated environment that preserves proxy settings.
- Package refresh, upgrade, autoremove and service-manager reload tasks bypass the ledger cache.
- Apt tasks run requested autoremove after distribution upgrades in apply and check mode.
- Agents require one successful handshake before executing batches and reject duplicate handshakes.
- Agents finish the current iteration and flush the ledger on SIGTERM or SIGINT during a batch.
- Sensitive-resource tracking treats parent traversal and relative remote paths conservatively.
- Block check mode reaches nested tasks, rescue/always tasks and controller pauses; child overrides take precedence.
- Workspace lints from the operator's repository standard, clippy configuration.
- Crates moved to `crates/<group>/<package>` (repository policy).
- Dependencies updated to their newest versions.
- Toolchain: Rust 1.99.1, operator mise layout with mbx as the cargo wrapper, lockfile v4; uv removed.
- CI regenerated with velnor-actions 0.1.75; tool versions now come from `mise.toml`.

### Removed

- Copies of `schema/Ruxel.pkl` beside fixtures, and the acceptance tools' `--schema` options.
- Duplicate legacy loop-result reason field; empty loops expose `skip_reason`.
- YAML and INI configuration input and MiniJinja authoring.
- Ansible parity tooling, Ansible-era docs, and agent modules no schema reaches; recoverable from tag `ansible-parity-final`.
- `justfile`; `mise run check` replaces `just check`. The Python Ansible-parity suites no longer run by default.

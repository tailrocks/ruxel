# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `hk.pkl` git hooks (mise, gitleaks, cargo fmt, check, clippy, deny, rumdl).
- Initial research implementation and closed-workload CLI/agent surface.

### Changed

- Toolchain: Rust 1.99.1, operator mise layout with mbx as the cargo wrapper, lockfile v4; uv removed.
- CI regenerated with velnor-actions 0.1.74; tool versions now come from `mise.toml`.

### Removed

- `justfile`; `mise run check` replaces `just check`. The Python Ansible-parity suites no longer run by default.

# Local manifest acceptance

Standalone safe Rust port for four reviewed manifest scenarios:
`check-semantics`, `multihost`, `controller-delegation`, and `restart-actions`. Each contract must
exactly match the frozen reviewed contract in `contracts.json`; fixture and
baseline oracle hashes are also pinned before any subprocess starts. Unsupported
scenarios fail closed. Omitting `--scenario` selects all 14, so the command is
expected to fail while 10 scenarios have no adapter.

The restart adapter requires effective UID 0 and executes the exact pinned
fixture using a disposable payload. Run it inside an isolated Linux guest with
no network, no host mounts, and all capabilities dropped. It checks all five
phases, file ownership, mode and content, all 36 ordered command results, and
preview non-mutation. The fixture executes a synthetic no-op script; it does not
test real service restarts.

```sh
cargo test --locked --offline --manifest-path tools/acceptance-manifest/Cargo.toml
cargo run --locked --offline --manifest-path tools/acceptance-manifest/Cargo.toml -- \
  --root . --ruxel target/debug/ruxel --agent target/debug/ruxel-agent \
  --scenario check-semantics --scenario multihost --scenario controller-delegation
```

The actual CLI and local agent run with disposable temporary payloads only. This
is not evidence for privileged workloads or full matrix parity. The candidate
`acceptance-local` task runs both commands; aggregate execution remains blocked
as documented in `docs/RUST-ACCEPTANCE-WIRING.md`. Former driver sources remain
until coordinated removal.

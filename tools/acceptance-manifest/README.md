# Local manifest acceptance

Standalone safe Rust port for three reviewed unprivileged manifest scenarios:
`check-semantics`, `multihost`, and `controller-delegation`. Each contract must
exactly match the frozen reviewed contract in `contracts.json`; fixture and
baseline oracle hashes are also pinned before any subprocess starts. Unsupported
scenarios fail closed. Omitting `--scenario` selects all 14, so the command is
expected to fail while 11 scenarios have no adapter.

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

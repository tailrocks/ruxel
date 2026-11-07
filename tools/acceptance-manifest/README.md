# Local manifest acceptance

Standalone safe Rust port for three reviewed unprivileged manifest scenarios:
`check-semantics`, `multihost`, and `controller-delegation`. Each contract must
exactly match the frozen reviewed contract in `contracts.json`; fixture and
baseline oracle hashes are also pinned before any subprocess starts. Unsupported
scenarios fail closed. Omitting `--scenario` selects all 14, so the command is
expected to fail while 11 scenarios have no adapter.

Restart lifecycle unit tests pin its unchanged workload and observation contract,
check synthetic metadata/results, and exercise a fake lifecycle's phase ordering
and preview non-mutation. This is groundwork only: `restart-actions` remains
unsupported and its CLI guard test verifies rejection before launching a client.
No service restart or full restart-scenario acceptance is established by these tests.

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

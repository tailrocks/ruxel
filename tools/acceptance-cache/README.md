# Local cache and synthetic provenance acceptance

Standalone safe Rust port of the local cache and synthetic secret provenance
checks. It invokes the real CLI and agent with disposable paths, synthetic
values, local mode, and no credentials. Build the binaries first, then run:

```sh
cargo test --locked --offline --manifest-path tools/acceptance-cache/Cargo.toml
cargo run --locked --offline --manifest-path tools/acceptance-cache/Cargo.toml -- \
  --ruxel target/debug/ruxel --agent target/debug/ruxel-agent --schema schema/Ruxel.pkl
```

The tests cover ledger hit, bypass, drift, source changes and noLog; redaction
through lookups, transforms, facts, commands, files, environment, templates,
conditions, failures and handler failures; public output; and state leakage.
This crate is not yet wired into CI and does not establish full matrix parity.

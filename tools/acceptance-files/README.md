# Local file acceptance

Standalone Rust port of the local file lifecycle and native template source checks.
It invokes the supplied Ruxel CLI and agent in local mode with disposable temporary
paths; it does not use a server. Run from the repository root after building both:

```sh
cargo test --locked --offline --manifest-path tools/acceptance-files/Cargo.toml
cargo run --locked --offline --manifest-path tools/acceptance-files/Cargo.toml -- \
  --ruxel target/debug/ruxel --agent target/debug/ruxel-agent \
  --schema schema/Ruxel.pkl --asset testdata/workloads/pr66/assets/template.pkl
```

The fixture checks fresh application, idempotence, drift repair, and check/diff
nonmutation. It also checks source and scope changes against a pinned template.
These tests do not establish parity for the complete files-content workload or
any privileged scenario. The candidate `acceptance-local` task runs both commands;
aggregate execution remains blocked as documented in `docs/RUST-ACCEPTANCE-WIRING.md`.

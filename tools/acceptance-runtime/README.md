# Local runtime acceptance

This standalone safe Rust runner exercises the real Ruxel CLI and agent using
temporary local fixtures. The candidate `acceptance-local` task runs its tests
and runner; aggregate execution remains blocked as documented in
`docs/RUST-ACCEPTANCE-WIRING.md`. The runner covers scoped
variables and environment, handler ordering and notification, convergence,
check/diff non-mutation, drift repair, and per-host cross-play private facts with redaction checks. It also covers control flow, failing handlers, nested loops and per-item
variables, including wire-level item labels.

```sh
cargo run --locked --offline --manifest-path tools/acceptance-runtime/Cargo.toml -- \
  --ruxel /absolute/path/to/ruxel --agent /absolute/path/to/ruxel-agent
```

Use `--root /path/to/checkout` to select the checkout that supplies the lazy-layer workloads. Linux and macOS
are supported. A dedicated process group is terminated on every outcome;
descendants that start a separate process group or session are outside that
cleanup boundary. The runner uses only synthetic local data and no servers.

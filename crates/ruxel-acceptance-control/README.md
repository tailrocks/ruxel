# Local control-flow acceptance

This standalone safe Rust runner ports the registered retry, check-mode semantics,
and task check-mode override scenarios. It drives the supplied real Ruxel CLI and
agent over local transport using temporary synthetic files. It does not connect to
servers. The crate is intentionally separate from the main Cargo workspace and
has tests and runner commands in the candidate `acceptance-local` task. Aggregate
execution remains blocked as documented in `docs/RUST-ACCEPTANCE-WIRING.md`.

From the repository root, using already-built matching binaries:

```sh
cargo run --locked --offline --manifest-path tools/acceptance-control/Cargo.toml -- \
  --ruxel /absolute/path/to/ruxel --agent /absolute/path/to/ruxel-agent
```

Use `--root /path/to/checkout` to select a checkout explicitly. The default is
the checkout in which the runner was compiled. The runner checks the pinned
SHA-256 of `testdata/workloads/pr66/check-semantics.pkl` before running it;
changes require review before updating the hash. Each run uses a new temporary
directory and has bounded CLI execution time. On Linux and macOS it runs the
CLI in a dedicated process group, terminates that group on completion or timeout,
and reaps the CLI process. A descendant that deliberately starts a separate
session or process group is outside that cleanup boundary.

The scenarios cover a registered retry, plan/check/diff predictions and handler
notification, fresh and converged apply, drift repair, and default/true/false
check-mode overrides. They do not replace the rest of the acceptance suite,
exercise remote hosts, or establish privileged provider behavior.

Unit checks can be run independently:

```sh
cargo test --locked --offline --manifest-path tools/acceptance-control/Cargo.toml
cargo clippy --locked --offline --manifest-path tools/acceptance-control/Cargo.toml --all-targets -- -D warnings
```

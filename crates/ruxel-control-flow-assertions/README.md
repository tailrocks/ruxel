# Control-flow observation assertions

This standalone safe Rust library validates externally supplied public events,
all three public output streams, synthetic files and retry counts. It does not
execute a scenario or establish that control-flow acceptance is supported.

The caller supplies `assert_observations` with an exact account-name-to-UID
resolver. It must reject unknown names and must not return a UID based on the
observed file; the assertion compares the resolved UID to actual file metadata.
This explicit dependency avoids account-directory or network lookups inside the
library. Symlink leaves and paths resolving outside the fixture root are rejected.

```sh
cargo test --locked --offline --manifest-path tools/control-flow-assertions/Cargo.toml
cargo clippy --locked --offline --manifest-path tools/control-flow-assertions/Cargo.toml --all-targets -- -D warnings
```

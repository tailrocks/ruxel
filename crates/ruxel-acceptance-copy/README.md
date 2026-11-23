# Local copy and repeated-action acceptance

This standalone safe Rust runner ports the binary copy, `force=false` copy, and
repeated shell-action acceptance scenarios. It drives already-built matching
Ruxel CLI and agent binaries over local transport using temporary synthetic files.
It never connects to servers and is intentionally separate from the workspace
and has test and runner commands in the candidate `acceptance-local` task.
Aggregate execution remains blocked as documented in `docs/RUST-ACCEPTANCE-WIRING.md`.

From the repository root:

```sh
cargo run --offline --locked --manifest-path tools/acceptance-copy/Cargo.toml -- \
  --ruxel /absolute/path/to/ruxel --agent /absolute/path/to/ruxel-agent
```

The fixtures import `modulepath:/ruxel/engine/Engine.pkl`, which the `ruxel` binary
under test serves from its own build.

The binary scenario verifies exact bytes, file mode, convergence, drift repair,
source replacement, empty source, check/diff non-mutation, and noLog censorship.
The force scenario verifies metadata repair without content replacement, source
change preservation, missing-destination recreation, and check/diff predictions.
The action scenario verifies 36 ordered writes on each of two applies and no
writes during check/diff.

CLI invocations have deadlines and use a safe Rust process-group sentinel copied
from the independently reviewed control runner. Cleanup terminates the group on
success, failure, or timeout and bounds direct-child reaping. Descendants that
intentionally detach into another process group or session are outside that
boundary. Output is drained concurrently into memory, capped at 8 MiB per stream;
overflow triggers process-group cleanup. Each pipe collection has a five-second
deadline after cleanup. These synthetic scenarios do not establish remote or
privileged provider behavior.

```sh
cargo test --offline --locked --manifest-path tools/acceptance-copy/Cargo.toml
cargo clippy --offline --locked --manifest-path tools/acceptance-copy/Cargo.toml --all-targets -- -D warnings
```

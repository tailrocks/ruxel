# Local plan and operation coverage gates

This standalone safe Rust runner checks the Pkl operation union against the
agent dispatch and controller admission and execution routes. This is a
structural guard, not proof of provider behavior. It then runs all 14 synthetic
workloads through the real local CLI in plan mode and checks planned events,
recaps, and payload non-mutation. The entire workload tree is SHA-256 pinned;
fixture changes require reviewing and updating `pins.txt`. It does not run
against servers or establish apply behavior.

```sh
cargo run --locked --offline --manifest-path tools/acceptance-plan/Cargo.toml -- \
  --ruxel /absolute/path/to/ruxel --agent /absolute/path/to/ruxel-agent
```

Optional `--root` selects the checkout. Linux and macOS are supported. A
bounded-output process group is terminated on each outcome; descendants that
create a separate process group or session are outside that cleanup boundary.
Tests run with `cargo test --locked --offline --manifest-path
tools/acceptance-plan/Cargo.toml`. This crate is not wired into CI.

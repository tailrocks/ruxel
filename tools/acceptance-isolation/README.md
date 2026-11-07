# Local isolation acceptance

Safe Rust replacements for the four PR66 host-isolation, handshake-isolation,
post-handshake-isolation, and partial-recap acceptance drivers. This standalone
crate runs the supplied real CLI and agent through local pipes only. It creates
synthetic local inventories and never invokes an SSH or network transport.

```sh
cargo test --locked --offline --manifest-path tools/acceptance-isolation/Cargo.toml
cargo clippy --locked --offline --manifest-path tools/acceptance-isolation/Cargo.toml --all-targets -- -D warnings
cargo run --locked --offline --manifest-path tools/acceptance-isolation/Cargo.toml -- \
  --ruxel target/debug/ruxel --agent target/debug/ruxel-agent
```

Pkl must be on PATH. `--root PATH` optionally selects the checkout containing
`schema/Ruxel.pkl`; it otherwise uses this crate's checkout. Linux and macOS are
supported. The runner and all authored helpers forbid unsafe Rust. No interpreter
or runtime compiler is required.

Assertions retained from the original drivers:

- Four distinct host facts, exactly two concurrent instrumented commands, and a
  barrier between plays. Every instrument event is appended under a standard
  library file lock. Its journal position replaces the original monotonic
  timestamp, preserving the linear order of process start/end observations.
- Two distinct agents are drained on shutdown, completed progress remains in
  each recap, and both nonzero shutdown exits become structured unreachable
  events.
- Both host orderings are tested for failure before and after the real agent's
  complete HelloAck. The healthy host completes both command plays, both recaps
  remain, and the failed host is launched exactly once. The native pipe proxy
  validates and bounds the first frame before forwarding it and killing/reaping
  its child.
- Ordinary and noLog execution errors retain earlier per-host copy progress,
  allow the healthy sibling to continue, and keep the synthetic sensitive marker
  out of stdout, stderr, and the event log.

Each CLI run is bounded to 25 seconds, with at most 8 MiB retained per output
stream. An unreaped sentinel anchors its process group until cleanup on success,
failure, output overflow, or timeout. Descendants that deliberately create new
process groups or sessions remain outside that cleanup boundary. Fixture journals
are limited to 1 MiB, and HelloAck frames to 4 MiB.

The candidate `acceptance-local` task runs both tests and runner. Aggregate
execution remains blocked as documented in `docs/RUST-ACCEPTANCE-WIRING.md`.
Former driver sources remain until coordinated removal.

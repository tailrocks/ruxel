# Local controlling-terminal acceptance

This standalone Rust suite runs the real CLI and local agent with a disposable
temporary Pkl fixture. It requires Unix PTYs on Linux or macOS. It never uses SSH
or real credentials. The crate has a separate lockfile and forbids unsafe code.

From the repository root, after building the CLI and agent:

```sh
cargo test --offline --locked --manifest-path tools/acceptance-pty/Cargo.toml
cargo run --offline --locked --manifest-path tools/acceptance-pty/Cargo.toml -- \
  --ruxel target/debug/ruxel --agent target/debug/ruxel-agent
```

The first command checks that partial JSON times out after the child actually
produces output, then kills and reaps its process group. It also checks cleanup
of a live pipe-holding descendant after its parent has exited. The second command runs
all seven integration cases: Enter, typed input, EOF, direct SIGINT, terminal
Ctrl+C, check and diff. Every apply case must block before input; successful
input cases must continue to the real Copy provider. Terminal Ctrl+C requires
the CLI to own the foreground terminal process group and must prevent Copy.
Typed input intentionally returns empty `user_input`, preserving the frozen
pause contract. Check and diff must skip pause and leave the marker absent.

Reads drain both nonblocking output pipes, preserve partial JSON and enforce
separate 10-second prompt/completion deadlines with a 1 MiB buffer limit.
Cleanup always kills the child process group, including after the CLI has
exited, then reaps with a bounded wait. macOS transient group permission errors
receive at most two seconds of retries. Cleanup failure fails the suite.

The candidate `acceptance-local` task runs both commands; building this crate
alone does not exercise the real CLI integration cases. Aggregate execution
remains blocked as documented in `docs/RUST-ACCEPTANCE-WIRING.md`.

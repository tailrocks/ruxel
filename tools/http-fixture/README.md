# Disposable loopback HTTP payload fixture

A standard-library-only safe Rust replacement for the payload-serving command
in `testdata/workloads/pr66/system-surface.pkl`. It always binds IPv4 loopback;
there is no option to bind a remote interface. GET and HEAD serve exactly
`/payload-source.txt` from the supplied directory, including binary or empty
content. The file is read on every request, so later source changes are visible.
Other paths are refused, and no directory listing or general web-server surface
is provided. The scenario uses no other endpoint.

```sh
cargo test --locked --offline --manifest-path tools/http-fixture/Cargo.toml
cargo clippy --locked --offline --manifest-path tools/http-fixture/Cargo.toml --all-targets -- -D warnings
cargo build --release --locked --offline --manifest-path tools/http-fixture/Cargo.toml
./tools/http-fixture/target/release/ruxel-http-fixture --directory /isolated/payload --port 18080
```

The disposable-VM harness must build this crate for its fixture VM's OS and
architecture and stage that executable at `__RUN_ROOT__/ruxel-http-fixture`
before executing system-surface. Keep the run directory and staged artifact
owned and writable only by the harness. The start task checks the launched process is alive and its exact bound-address
and PID readiness line is present before making a readiness request. A failed
bind cannot be hidden by an unrelated listener. It rejects missing,
nonexecutable, wrong-architecture, or incompatible-version artifacts before
starting the server. It retains the original PID file, curl readiness checks,
retry task, GetUrl task, and stop task. No runtime compiler or interpreter is
required on the fixture VM. **Artifact staging remains an unresolved harness
dependency; this change does not establish VM execution readiness.**

Stage the artifact before the first apply. The workload's server-start and stop
tasks inherit check mode and are skipped during check and diff. Preview does not
test HTTP readiness or connectivity.
Preparation therefore belongs in the disposable harness's separately authorized
setup phase. The current local manifest runner has no system-surface adapter;
its successful three-case run does not perform or verify this staging step.

The complete system-surface workload modifies host accounts, firewall, mounts,
services, and authorized keys. Never run it locally or on real servers. Native
unit/integration checks here use only isolated loopback listeners and synthetic
files. They do not establish package, service, firewall, or disk parity.

Headers are capped at 8 KiB with an absolute two-second request-header deadline.
Payloads are capped at 8 MiB, and each socket write waits at most two seconds. The fixture accepts one connection at a time. Its root and
payload are trusted harness-owned paths; it is not designed to withstand a
concurrent administrator replacing those paths. Symlinks escaping the root are
refused. Default process termination handles the scenario's existing stop task.

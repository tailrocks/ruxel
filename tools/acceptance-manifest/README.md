# Local manifest acceptance

Standalone safe Rust port for six reviewed manifest scenarios:
`check-semantics`, `multihost`, `controller-delegation`, `restart-actions`, `control-flow`, and `postgresql-ownership`. Each contract must
exactly match the frozen reviewed contract in `contracts.json`; fixture and
baseline oracle hashes are also pinned before any subprocess starts. Unsupported
scenarios fail closed. Omitting `--scenario` selects all 14, so the command is
expected to fail while eight scenarios have no adapter.

The control-flow adapter requires the same isolated Linux guest as the restart
adapter. It runs a private in-memory observer of the real controller and agent
and separately checks the real CLI's complete censored event sequence and log.
The fixture's pause is tested with noninteractive stdin; interactive terminal
behavior is not covered.

The restart adapter requires effective UID 0 and executes the exact pinned
fixture using a disposable payload. Run it inside an isolated Linux guest with
no network, no host mounts, and all capabilities dropped. It checks all five
phases, file ownership, mode and content, all 36 ordered command results, and
preview non-mutation. The fixture executes a synthetic no-op script; it does not
test real service restarts.

```sh
cargo test --locked --offline --manifest-path tools/acceptance-manifest/Cargo.toml
cargo run --locked --offline --manifest-path tools/acceptance-manifest/Cargo.toml -- \
  --root . --ruxel target/debug/ruxel --agent target/debug/ruxel-agent \
  --scenario check-semantics --scenario multihost --scenario controller-delegation
```

The actual CLI and local agent run with disposable temporary payloads only. This
is not evidence for privileged workloads or full matrix parity. The candidate
`acceptance-local` task runs both commands; aggregate execution remains blocked
as documented in `docs/RUST-ACCEPTANCE-WIRING.md`. Former driver sources remain
until coordinated removal.

The PostgreSQL adapter starts a new disposable PostgreSQL cluster with TCP
listening disabled and a private Unix socket. It requires a network-isolated
Docker guest without host mounts, PostgreSQL server and client packages, a
`postgres` account, and `RUXEL_POSTGRES_FIXTURE=isolated`. It checks explicit
catalog ownership and grants and captures catalog state across preview runs.

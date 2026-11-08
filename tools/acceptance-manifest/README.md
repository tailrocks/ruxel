# Local manifest acceptance

Standalone safe Rust port for five reviewed manifest scenarios:
`check-semantics`, `multihost`, `controller-delegation`, `restart-actions`, and `control-flow`. Each contract must
exactly match the frozen reviewed contract in `contracts.json`; fixture hashes
are also pinned before any subprocess starts. Unsupported
scenarios fail closed. Omitting `--scenario` selects all 14, so the command is
expected to fail while nine scenarios have no enabled adapter.

For controller delegation, the actual apply runs must pass the fixture's
assertion on the delegated command's registered stdout. Separate check and diff
runs must skip the command and fail that assertion, report the corresponding
recap, preserve censorship, and leave the payload and fixture tree unchanged.

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

The PostgreSQL and become-environment adapters are disabled pending a reliable
way to establish that `/tmp` is not a host bind mount. Their previous isolated
guest results do not prove that this guard would reject all host mounts.

The PostgreSQL adapter starts a new disposable PostgreSQL cluster with TCP
listening disabled and a private Unix socket. It requires a network-isolated
Docker guest without host mounts, PostgreSQL server and client packages, a
`postgres` account, and `RUXEL_POSTGRES_FIXTURE=isolated`. It checks explicit
catalog ownership and grants and captures catalog state across preview runs.


The become-environment adapter creates and modifies an account, so it requires
an ephemeral Docker guest with no external network or writable host mounts and
`RUXEL_BECOME_FIXTURE=isolated`. It verifies account metadata, environment
results, shell drift and repair, and preview filesystem state.
Its previews run after drift repair. A preview against the drifted shell fails
the fixture's environment assertion, so drifted preview success is not covered.

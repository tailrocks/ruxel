# Local manifest acceptance

Standalone Rust manifest acceptance with five local adapters and three host-orchestrated Docker adapters:
`check-semantics`, `multihost`, `controller-delegation`, `restart-actions`, `control-flow`, `postgresql-ownership`, `become-environment`, and `files-content`. Each contract must
exactly match the frozen reviewed contract in `contracts.json`; fixture hashes
are also pinned before any subprocess starts. Unsupported
scenarios fail closed. Omitting `--scenario` selects all 14, so the command is
expected to fail while six scenarios have no enabled adapter.

The files-content adapter runs the pinned workload and pinned source and template
assets in the isolated Docker guest. A guest-local synthetic systemctl records
the fixture's ssh restart handler without restarting an actual service. Fresh and
repeated apply, check and diff over injected drift, and drift repair verify file
observations and preview nonmutation. Repeated apply is not globally unchanged:
several fixture tasks intentionally rewrite the same file in sequence. The
fixture's pending frozen-baseline newline comparison remains separate evidence.

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

The local task also runs the `secret_dedup` example against the actual controller
and agent. It uses a temporary fake `op` executable and synthetic fields to check
same-item deduplication and output redaction in check and diff, including a
wrong-field negative control. It does not contact 1Password or establish full
performance-snapshots acceptance.

The actual CLI and local agent run with disposable temporary payloads only. This
is not evidence for privileged workloads or full matrix parity. The candidate
`acceptance-local` task runs both commands; aggregate execution remains blocked
as documented in `docs/RUST-ACCEPTANCE-WIRING.md`. Former driver sources remain
until coordinated removal.

PostgreSQL and privilege escalation run only through host-side Docker
orchestration. Each selected scenario creates a fresh container from a complete
local `sha256:` image ID. The launcher uses a fixed local Unix socket and an
empty Docker client configuration, clears inherited Docker environment settings,
and never pulls images. There is no guest adapter mode or environment-marker
bypass. Native host binaries are rejected; supply matching Linux ELF controller
and agent binaries built from the reviewed source.

Before starting the dormant container, and before every subsequent execution or
input transfer, the launcher checks its immutable image, namespace, capability,
mount and resource configuration. Bind mounts, named volumes, image-declared
volumes, custom devices, exposed ports, privileged mode and external networking
are refused. `/tmp` is a runtime-created tmpfs. Inputs are copied by bounded
stdin streaming, never mounted from the host. Executables reside on the
container's disposable image layer; fixture data stays in the private tmpfs.
Only CHOWN, DAC_OVERRIDE, FOWNER, SETGID and SETUID capabilities are admitted;
SYS_ADMIN is never granted. Container removal runs after success and failure,
with a removal retry during unwinding. AutoRemove and a 900-second dormant process provide best-effort expiry after
launcher interruption. This is not a hard lifetime guarantee against guest-root
interference; normal explicit container removal remains authoritative.

The already provisioned image must contain PostgreSQL server/client tools,
`runuser`, account-management tools, Bash and GNU coreutils/findutils. Package
provisioning belongs to the separately reviewed image build. Run the host driver:

```sh
cargo run --locked --offline --manifest-path tools/acceptance-manifest/Cargo.toml \
  --features ruxel-engine/pure -- \
  --root . --ruxel /absolute/Linux/ruxel --agent /absolute/Linux/ruxel-agent \
  --fixture-image sha256:COMPLETE_LOCAL_IMAGE_ID \
  --scenario postgresql-ownership --scenario become-environment
```

PostgreSQL uses a new cluster, a private Unix socket and disabled TCP listeners.
Its catalog assertions check ownership, roles, database/schema/default grants,
and absence of the deliberately predicted schema. Convergence requires unchanged
actual operations while that explicit `checkMode = true` schema task continues
to predict creation. Grant-removal negative controls must fail the observation;
check/diff compare catalog state and fixture filesystem snapshots.

The account adapter checks account metadata, exact become environment results,
shell drift/repair and converged unchanged results. Filesystem snapshots hash
contents, links, ownership, modes, inode and modification/change timestamps for
`/etc`, `/var/log`, payload, workload and schema. Its normal previews run after
repair. Drifted previews intentionally fail the environment assertion with exit
8 and must preserve those snapshots; this remains a documented contract limit.

# Plan 018: Consolidate duplicated code (runner, varint, registries, constants)

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. Refactors
> must not change behavior (guarded by the existing + new tests).
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/modules/ crates/ruxel-proto/src/frame.rs crates/ruxel/src/transport.rs crates/ruxel-core/src/modules.rs`
> If any changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: MED (the varint-codec unification touches the transport hot path)
- **Depends on**: none (best landed after 014's `write_atomic` helper and 016's
  module tests exist, so the runner refactor has coverage; coordinate)
- **Category**: tech-debt
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Four duplication/drift hazards, built up across fast sessions:
1. **DEBT-02**: ~14–20 hand-rolled `Command::new(...).output()` + stderr-capture
   copies across agent modules. Fixing one behavior (surface exit code, capture
   stdout on failure) is a ~14-site change.
2. **DEBT-07**: the varint frame codec is implemented **twice** — sync in
   `ruxel-proto/frame.rs` and async in `transport.rs` — and has **already
   drifted** (the sync one handles `ErrorKind::Interrupted`; the async copy does
   not). Two wire parsers that must stay byte-compatible by hand.
3. **DEBT-05**: three module registries (`ruxel-core` `MODULES`, agent
   `execute()` dispatch, ledger `probe_for`) are hand-synced; a module in the
   core registry with no agent dispatch arm fails only at **runtime** on a live
   host.
4. **DEBT-08**: coupled magic literals live apart — `/var/lib/ruxel` in two
   crates, `/etc/sysctl.conf` defaulted independently in `sysctl.rs` (writer) and
   `ledger.rs` (fingerprint reader); if they diverge the ledger reads a file the
   module never wrote and caching silently never verifies.

## Current state

**A. Command runner:** repeated block across
`crates/ruxel-agent/src/modules/{iptables,sysctl,get_url,filesystem,lvg,lvol,
mount,apt,apt_repository,misc,systemd,postgresql}.rs`:
```rust
let out = std::process::Command::new(bin).args(args).output().map_err(...)?;
if !out.status.success() { return Err(format!("... {}", String::from_utf8_lossy(&out.stderr).trim())); }
```
Counts (grep): `String::from_utf8_lossy(&*.stderr)` ~14×, `Command::new` ~26×,
`map_err(|e| e.to_string())` ~35×. `become_command` (`modules/mod.rs:147-169`)
builds a `Command` but never runs/captures — so every site re-rolls run+check.
(The file-attr layer is *already* shared via `apply_attrs` — only process
spawning is duplicated.)

**B. Varint codec:** `crates/ruxel-proto/src/frame.rs:14-65` (sync
`read_frame`/`write_frame`) and `crates/ruxel/src/transport.rs:379-419` (async
versions; `:379` comment "same wire format as ruxel_proto::frame"). Drift:
`frame.rs:38` handles `Interrupted`, the async copy does not. `MAX_FRAME_LEN` is
shared already (good).

**C. Registries:** `crates/ruxel-core/src/modules.rs` `MODULES` (36 surfaces;
its test only asserts `len == 36`), agent `crates/ruxel-agent/src/modules/mod.rs:96-130`
`execute()` dispatch (~31 arms), ledger `probe_for` (`ledger.rs:157-211`, 5
groups). Nothing maps core→agent→probe; a missing agent arm surfaces as
`"module {other:?} is not implemented in this agent build"` at runtime.

**D. Constants:** `AGENT_DIR = "/var/lib/ruxel/agent"` (`transport.rs:25`) and
`state_dir()` → `/var/lib/ruxel` (`crates/ruxel-agent/src/main.rs:45-49`);
`/etc/sysctl.conf` defaulted in `sysctl.rs:26` and `ledger.rs:201`; the two 30s
timeouts (agent orphan guard `main.rs:92`, controller HelloAck `transport.rs:250`).

**Convention**: agent stays dependency-light (`Result<_, String>`, no async);
`ruxel-proto` is the shared crate between controller and agent — the ideal home
for a shared varint state machine.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Build | `cargo build --workspace` | exit 0 |
| All tests | `cargo nextest run` | pass (no regressions) |
| Transport gate (on-VM) | `RUXEL_TEST_SSH_DEST=... cargo test ... transport_gate -- --ignored` | operator/fixture |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**: a shared runner in `modules/mod.rs`; a shared varint state machine
in `ruxel-proto`; a registry cross-check test; shared constants.

**Out of scope**:
- Behavior changes (pure consolidation). The varint unification especially must
  produce byte-identical wire output.
- The dead proto surface (plan 003 Step 5 handles the comment).
- Splitting god modules (`engine.rs`/`scheduler.rs`) — the auditor found them
  cohesive; not worth it.

## Git workflow

- Branch: `advisor/018-consolidation`
- Commit per lettered item (independent). Land the varint one (B) with extra care.
- Do NOT push/PR unless instructed.

## Steps

### Step A: Shared command runner

Add to `modules/mod.rs`:
```rust
pub(super) fn run_checked(mut cmd: std::process::Command) -> Result<std::process::Output, String> { ... }
pub(super) fn run_ok(cmd: std::process::Command) -> Result<bool, String> { ... } // status.success()
```
`run_checked` runs `.output()`, and on non-zero returns
`Err(format!("{program} {args}: {stderr}"))` — matching the current message
shape (so no output changes). Route the ~14 raw sites through it. Keep
`become_command` as the `Command` builder; feed its output into `run_checked`.
**Do not** change `psql` in `postgresql.rs` (it streams SQL on stdin — a special
case; leave it, or add a stdin-aware variant only if trivial).

**Verify**: `cargo nextest run -p ruxel-agent` → the module tests from plan 016
(and existing) still pass with identical error strings; `cargo clippy -p
ruxel-agent --all-targets -- -D warnings` → 0.

### Step B: Unify the varint codec

Put the length-prefix state machine in `ruxel-proto` as a pure, I/O-free helper
(byte-in → `Option<len>` or a small `VarintDecoder` struct; plus an encoder for
the length prefix). Rewrite both `frame.rs`'s sync `read_frame`/`write_frame`
and `transport.rs`'s async versions to use it, so there is **one** algorithm.
Ensure the async path now also handles `Interrupted` (the drift the sync path
already handles). Keep `MAX_FRAME_LEN` shared.

**Verify**: `cargo nextest run -p ruxel-proto` → the existing frame tests (and
plan 017's new edge tests, if landed) pass; `cargo build -p ruxel-cli` → 0;
add a test asserting the async and sync paths produce identical bytes for the
same message (encode with one, decode with the other). The **on-VM transport
gate** must still connect (operator/fixture) — note it; a wire-format regression
here breaks every run.

### Step C: Registry cross-check test

Add a test (in `ruxel-agent` or a shared integration test) that iterates
`ruxel_core::modules::MODULES`, subtracts the known controller-side set
(`assert`, `debug`, `fail`, `pause`, `set_fact` — which have no agent arm), and
asserts every remaining name resolves in the agent `execute()` dispatch. If
`execute` isn't directly enumerable, expose a `pub fn is_implemented(module:
&str) -> bool` in the agent that mirrors the match, and test against it (keep the
two in sync via a single source — e.g. `is_implemented` and `execute` both match
the same list). Document the cacheable subset next to `probe_for`.

**Verify**: `cargo nextest run` → the cross-check test passes today (proves the
registries are currently in sync) and would fail if a module were added to core
without an agent arm.

### Step D: Shared constants

Create a small shared consts module (in `ruxel-proto` or a new
`ruxel-core::paths`, whichever both crates can see — `ruxel-proto` is shared by
both) for `/var/lib/ruxel` root and the default `/etc/sysctl.conf`. Use it in
`transport.rs:25`, `main.rs:45-49`, `sysctl.rs:26`, `ledger.rs:201`. Give the
two 30s handshake timeouts a shared named const with a cross-referencing comment
(they must stay equal — the orphan guard must outlast the HelloAct wait). The
`/etc/sysctl.conf` pair is the sharpest: a single source removes the silent-cache
failure mode.

**Verify**: `grep -rn '"/var/lib/ruxel"\|"/etc/sysctl.conf"' crates/` → the
literals now come from one const (only the const definition remains a literal);
`cargo build --workspace` → 0.

### Step E: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- A: existing/016 module tests confirm identical error behavior post-runner.
- B: sync/async byte-identity test + existing frame tests + on-VM gate.
- C: registry cross-check test (passes now; guards future drift).
- D: grep confirms single-source constants; build proves it compiles.

## Done criteria

ALL must hold:

- [x] Agent modules use a shared `run_checked`/`run_ok`; the duplicated checked-run blocks are consolidated while output-sensitive/special-error sites remain explicit
- [x] One varint codec in `ruxel-proto` backs both sync (`frame.rs`) and async (`transport.rs`); the async path handles `Interrupted`; a byte-identity test passes
- [x] A registry cross-check test asserts every core module (minus controller-side) has an agent dispatch arm
- [x] `/var/lib/ruxel` and `/etc/sysctl.conf` come from single shared constants; the two handshake timeouts share a named const
- [x] `cargo nextest run` green; clippy/fmt clean; on-VM transport gate still connects
- [x] `plans/README.md` row for 018 updated

Local implementation and all hermetic gates completed 2026-07-16. The ignored
transport gate subsequently passed on a labeled disposable fixture; production
targets and real workload execution remained forbidden.

Disposable Hetzner fixture gate passed 2026-07-16: cold upload + handshake
1.905 s; warm cached handshake 983.5 ms with no re-upload.

## STOP conditions

Stop and report if:
- The varint unification changes any wire byte (the byte-identity test fails, or
  the on-VM gate breaks) — revert and report; wire compatibility is
  non-negotiable.
- Routing a module through `run_checked` changes its error string (some sites
  format differently) — preserve the exact message or note the intentional
  normalization in the PR.
- The registry cross-check reveals a module already missing an agent arm — that's
  a real bug; report it (don't just make the test pass by excluding the module).

## Maintenance notes

- After B, there is one place to fix any future framing edge case — the sync/
  async split was the root of the `Interrupted` drift.
- Reviewer: B is the risky one — scrutinize the byte-identity test and insist on
  the on-VM gate before merge.
- The registry cross-check (C) is cheap insurance against the "works in core,
  crashes on the host" class; keep it.

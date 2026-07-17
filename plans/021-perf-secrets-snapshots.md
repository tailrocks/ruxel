# Plan 021: Batch secret resolution and build agent system snapshots

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. Large perf
> work — land the two halves (secrets, snapshots) as separate commits; each is
> independently shippable and must preserve behavior/output.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel/src/secrets.rs crates/ruxel/src/commands/apply.rs crates/ruxel-core/src/engine.rs crates/ruxel-agent/src/modules/ crates/ruxel-agent/src/ledger.rs`
> If any changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED (secret resolution + agent snapshots both touch correctness-
  sensitive paths; guarded by parity/dry-secrets + on-VM gate)
- **Depends on**: 020 (the snapshot/probe-concurrency half benefits from
  whole-plan batching; the secrets half is independent and can land first)
- **Category**: perf
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Two gaps between ARCHITECTURE's stated design and the implementation, both
invisible in the secret-free 8-task benchmark but dominant on the real
setup-* workload:

1. **PERF-02 (secret resolution)**: `op_read` spawns **one `op read` subprocess
   per distinct field reference**, **lazily**, during per-task rendering — with
   no upfront/concurrent/grouped phase. Because the controller renders every
   task's params on **every** run (to compute the ledger key and ship the Plan),
   every `op read` fires every run, **including a converged no-op**. For
   setup-postgresql-nova's ~52 lookups that's up to ~52 serial `op` spawns
   (process + session + network each), realistically **~15–30 s** — versus §10's
   "52 lookups, deduplicated, ‖, warm `op` session ~1–3 s." This single gap can
   make a converged setup-* run slower than the 15-min Ansible baseline it
   exists to beat. (Two intent docs *claim* this is already solved — `GOAL.md`/
   `RESTORE.md` say "memoized to a handful of op calls" — the code contradicts
   them; this plan makes the claim true.)
2. **PERF-03 / PERF-04 (agent snapshots + probe concurrency)**: the agent has
   **no** batched system caches (ARCHITECTURE §5 unbuilt). It forks a subprocess
   per package (`dpkg-query`), per unit (`systemctl is-active`+`is-enabled`), and
   per SQL statement (`psql`), and the ledger **re-forks** on every verify. Even
   the headline ledger-cached path forks ~66 subprocesses just to *verify* the
   cache for a 65-task run. Probes are also evaluated sequentially in a
   single-threaded agent (§6's "concurrent whole-plan" is absent).

## Current state

**Secrets:**
- `crates/ruxel/src/secrets.rs:59-74` `op_read` — one `op read op://vault/item/field`
  per reference. An SSH item's private+public key = two subprocesses (§3.2 wanted
  **one** `op item get <item> --format json` serving both).
- `crates/ruxel-core/src/engine.rs:96-140` `MemoizedResolver` dedupes by exact
  reference string but the fetch is a plain synchronous call; two fields of one
  item = two cache misses = two subprocesses.
- No upfront/batched/concurrent resolve phase exists (grep: no `join_all`/
  `FuturesUnordered`/prefetch). Resolution is lazy inside the lookup fn during
  per-task rendering.
- `apply.rs:83-89` builds `MemoizedResolver::new(OpResolver)`; no "collect every
  distinct lookup, fetch concurrently before execution" step (ARCHITECTURE §3.2 /
  pipeline step 2).

**Agent snapshots:** no cache structs anywhere in the agent. Fork sites:
`apt.rs:188` (`dpkg-query -W` per package), `:202` (`apt-cache policy` per pkg
for latest); `systemd.rs:32,47` (`is-enabled`/`is-active` per unit);
`postgresql.rs:25-37` (`psql` per statement); ledger re-forks at `ledger.rs:233`
(dpkg), `:250-264` (systemctl). Agent `Cargo.toml` has **no** tokio/rayon (single-
threaded by design — deliberate static-musl footprint).

**Convention**: the deviation is *specified* (SEMANTICS §2: one consistent
secret snapshot per run) so batching secrets is not a semantic change. Snapshot
building must not change any module's reported status (pinned by fixture
goldens). The agent stays dependency-light — prefer a threadpool via std
`std::thread` over adding rayon/tokio, if concurrency is added at all.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Build | `cargo build --workspace` | exit 0 |
| Render parity (secrets path) | `cargo nextest run -p ruxel-core render_parity` | pass (unchanged) |
| Agent tests | `cargo nextest run -p ruxel-agent` | pass |
| On-VM gate + timing | `tools/fixtures/bless-gate.sh ...` + wall-clock | operator/fixture |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel/src/secrets.rs`, `crates/ruxel/src/commands/apply.rs`,
  `crates/ruxel-core/src/engine.rs` (the resolver + a pre-resolve phase).
- `crates/ruxel-agent/src/modules/{apt,systemd,postgresql}.rs`,
  `crates/ruxel-agent/src/ledger.rs` (snapshot-backed reads).
- A new agent `SystemState`/snapshot module.

**Out of scope**:
- apt adjacency batching (a separate ARCHITECTURE §5.3 optimization — not here).
- The content-addressed blob channel.
- Changing any module's **reported status** — snapshots are an internal speedup.
- Per-host parallelism — plan 022.

## Git workflow

- Branch: `advisor/021-perf`
- Commit the two halves separately: `perf(secrets): batch+concurrent op resolution`
  and `perf(agent): per-run dpkg/systemd/pg snapshots`.
- Do NOT push/PR unless instructed.

## Steps

### Step 1 (secrets, independent): Pre-resolve phase — group by item, fetch concurrently

Before `run` connects (in `apply::run`, after compile), walk the compiled tasks
(plan 020 gives you the compiled plan; if 020 hasn't landed, walk the playbook's
lookups) and collect every distinct `onepassword` lookup. **Group by
`(vault, item)`** and fetch each item **once** via `op item get <item>
--format json` (which returns all fields), on a bounded concurrent pool
(`tokio::task::spawn_blocking` for the `op` subprocess, `join_all` with a
semaphore cap of e.g. 8). Populate `MemoizedResolver`'s map keyed per field so
the later lazy renders hit the cache with **zero** further subprocesses.

Preserve exact field-selection semantics (section/field) and `no_log` redaction.
`--dry-secrets` must bypass `op` entirely (unchanged). Keep the memoization
guarantee (one consistent snapshot per run — SEMANTICS §2).

**Verify**: `cargo nextest run -p ruxel-core render_parity` → byte-identical
(the resolved values are unchanged; only the fetch strategy differs). Add a unit
test with a **fake** resolver counting calls: two fields of one item trigger
**one** item fetch, not two. On-VM (operator, `--dry-secrets` or the fake-lookup
gate): converged setup-* wall-clock drops from tens of seconds to the §10 budget.

### Step 2 (snapshots): Build a per-run `SystemState` in the agent

Add an agent `SystemState` struct built lazily once per run:
- **dpkg**: parse `/var/lib/dpkg/status` once (installed name→version) instead of
  forking `dpkg-query` per package; for `state: latest`, one batched
  `apt-cache policy <names...>` (or parse `/var/lib/apt/lists`) instead of per
  package.
- **systemd**: one `systemctl list-units --all --type=service` (+ unit-file
  states) parse instead of per-unit `is-active`/`is-enabled`. (A D-Bus
  connection is the ARCHITECTURE ideal but adds a dep; a single batched
  `systemctl` call is a dependency-free win — prefer it.)
- **postgresql**: keep one `psql` session per run where feasible (a single
  `psql` process fed multiple statements on stdin, or at least reuse the
  connection parameters) — the biggest win is avoiding a `runuser`+`psql` fork
  per statement.
Invalidate the relevant snapshot on any write (apt install, unit change, SQL
DDL). Inject `SystemState` into the modules and into the ledger's `Probe::verify`
so cache verification uses the snapshot, not per-name forks.

**Critical constraint**: module **reported status must not change** — the
snapshot is a faster way to compute the same answer. The fixture goldens and
on-VM bless-gate are the proof.

**Verify**: agent module tests (plan 016) still pass with identical results;
add a test that a snapshot answers N package/unit queries with **one** parse
(count forks via a shim, or assert the snapshot is built once). On-VM gate:
converged rerun `changed=0`, status-identical to Ansible, with a measurable drop
in subprocess count / wall-clock (operator measures).

### Step 3 (probe concurrency, optional/after 020): parallelize probe verify

Only after Step 2 (shared snapshot removes fork storms) and plan 020 (whole-plan
batches reach the agent): evaluate the ledger fingerprints for a batch
concurrently with a small `std::thread` worker pool, keyed by `task_id`.
File-hash probes (`sha256` of `copy`/`template` targets) parallelize cleanly.
Keep `panic=abort` semantics. **If the single-threaded snapshot path already
meets the seconds target (likely at current fixture sizes), defer this step**
and note it — sequential few-hundred stats over a shared snapshot is already
fast; measure before adding threads.

**Verify**: if implemented, results are identical and ordering by `task_id` is
preserved; a timing test shows the win only matters at scale. If deferred, note
it with the measured single-threaded number.

### Step 4: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- Secrets: call-counting fake resolver (one item fetch serves N fields);
  render-parity byte-identity preserved; `--dry-secrets` bypass intact.
- Snapshots: module tests unchanged; a "built once" assertion; on-VM bless-gate
  parity + subprocess-count/wall-clock drop (operator).
- Probe concurrency (if done): identical results, task_id ordering.

## Done criteria

ALL must hold:

- [x] Secrets are resolved in an upfront concurrent phase, grouped per 1Password item (one `op item get` per item); a call-counting test proves N-fields→1-fetch
- [x] Render parity is byte-identical (resolved values unchanged); `--dry-secrets` still bypasses `op`
- [x] The agent builds per-run dpkg/systemd(/pg) snapshots; per-name/per-unit/per-statement fork storms are gone; module reported status is unchanged
- [x] Synthetic on-VM bless-gate: converged rerun `changed=0`, status-identical to Ansible, measurably faster
- [x] Probe concurrency implemented OR explicitly deferred with the measured single-threaded number
- [x] `cargo nextest run` green; clippy/fmt clean
- [x] `plans/README.md` row for 021 updated; `GOAL.md`/`RESTORE.md`'s "memoized to a handful of op calls" claim is now actually true (note it for plan 002)

Probe concurrency is deferred: shared-snapshot single-thread measurement on
this development host parses 10,000 synthetic dpkg records and performs 10,000
ordered lookups in 16.824 ms. At current fixture scale, worker coordination
would dominate; `system_state::tests::single_threaded_snapshot_lookup_scale`
keeps the scale check reproducible. Revisit only if the disposable-host timing
gate contradicts this result.

Synthetic fixture timing on 2026-07-16 covered two fields from one dry-secret
item, apt/systemd snapshots, and two PostgreSQL checks sharing a session:
Ruxel 4.41 s vs Ansible 24.67 s (5.59×), both `changed=0`.

## STOP conditions

Stop and report if:
- Grouping lookups by item changes any resolved value (a section/field edge case)
  — render parity would break; report the exact reference and preserve semantics.
- A snapshot-backed read produces a **different status** than the per-name fork
  for any module (the on-VM gate shows a changed-set difference) — the snapshot
  is wrong; revert that module to the fork and report.
- Adding probe-concurrency requires a dependency (rayon/tokio) in the agent —
  prefer `std::thread`; if that's infeasible, defer Step 3 rather than bloat the
  static-musl agent.

## Maintenance notes

- The secrets half is the highest real-world win for setup-* runs and is
  independent — land it first.
- Reviewer: for snapshots, the non-negotiable is status-identity — a faster wrong
  answer is worse than a slow right one. Insist on the on-VM parity gate.
- After this + plan 020, ARCHITECTURE §5 (batched caches) stops being "NOT YET
  BUILT" — update plan 002's doc note.

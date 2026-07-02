# Plan 007: Flush the ledger on interrupt / connection loss

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/main.rs crates/ruxel-agent/src/ledger.rs`
> If either changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: 005 (uses its test harness patterns; not strictly required to compile)
- **Category**: bug (durability / correctness vs ARCHITECTURE §8)
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

`ARCHITECTURE.md §8` promises: on connection loss / operator Ctrl-C, "the agent
finishes the in-flight task, **writes the ledger**, exits. Rerun converges."
The in-flight task does finish (the agent reads at frame boundaries), but the
ledger is flushed **only** on an explicit `Done` message. A controller Ctrl-C
drops the SSH connection → the agent sees clean EOF → returns `0` **without
flushing** — so every fingerprint recorded during that run is discarded, and
the next run re-checks everything instead of hitting the promised
converged-no-op fast path (the 14.6× headline). This is a fail-safe gap (never
wrong state, just slow), but it defeats the feature on any interrupted run. The
fix is a few lines.

## Current state

`crates/ruxel-agent/src/main.rs`:
- `serve()` main loop (`:104-178`). The EOF branch (`:104-115`):
  ```rust
  let envelope: v1::Envelope = match read_frame(&mut stdin) {
      Ok(Some(env)) => env,
      // Clean EOF: controller went away (Ctrl-C, connection loss).
      Ok(None) => return 0,     // <-- ledger NOT flushed
      Err(e) => { log_event(...); return 64; }   // <-- also not flushed
  };
  ```
- The `Done` branch **does** flush (`:145-147`):
  ```rust
  Some(Msg::Done(_)) => { ledger.flush(); return 0; }
  ```
- `ledger` is created at `:102` (`let mut ledger = ledger::Ledger::load(&dir);`)
  and mutated by `execute_task` via `ledger.record(...)` (`:278`), setting
  `dirty`.
- `Ledger::flush` (`crates/ruxel-agent/src/ledger.rs:90-103`) is a no-op when
  `!dirty` and otherwise does an atomic tmp+rename — safe to call on any exit.
- The panic hook (`main.rs:20-39`) exits `70` without flushing (a panicked
  agent's in-memory state is untrustworthy, so **do not** flush there).

**Convention**: the agent is single-threaded, `Result<_, String>` + i32 exit
codes, no async. Flushing is idempotent and cheap.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Agent tests | `cargo nextest run -p ruxel-agent` | pass |
| Protocol test | `cargo nextest run -p ruxel-agent protocol` | pass |
| Build | `cargo build -p ruxel-agent` | exit 0 |
| Clippy/fmt | `cargo clippy -p ruxel-agent --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel-agent/src/main.rs` — the `serve()` EOF/error exit paths.
- `crates/ruxel-agent/tests/protocol.rs` — add a test proving flush-on-EOF.

**Out of scope**:
- The panic path (`:20-39`) — deliberately does **not** flush (untrustworthy
  state after a panic). Do not change it.
- Any probe/record logic (plans 005/006).
- Controller-side Ctrl-C handling (`crates/ruxel/src`) — the agent-side flush is
  the fix; controller signal handling is a separate concern (note it for the
  operator; the agent seeing EOF is the trigger that matters).

## Git workflow

- Branch: `advisor/007-ledger-flush`
- One commit: `fix(agent): flush ledger on clean EOF (ARCHITECTURE §8 durability)`
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Flush on clean EOF

Change the `Ok(None) => return 0` branch (`main.rs:110`) to flush first:
```rust
Ok(None) => { ledger.flush(); return 0; }
```

Decide on the `Err(e)` frame-error branch (`:111-114`, returns 64): a framing
error mid-stream means the connection is corrupt, but any fingerprints already
recorded from **completed** tasks are still valid (each was recorded only after
the module returned). Flushing there is also safe (tmp+rename, no partial
task). Add `ledger.flush();` before `return 64` as well, with a comment that
completed-task fingerprints are durable regardless of a later frame error.

**Verify**: `cargo build -p ruxel-agent` → 0; `git diff crates/ruxel-agent/src/main.rs`
shows only these two branches gained a `ledger.flush();` and the panic hook is
untouched.

### Step 2: Prove it with a protocol test

In `crates/ruxel-agent/tests/protocol.rs` (which already spawns the real agent
over pipes and manages `RUXEL_STATE_DIR`), add a test:
`ledger_flushes_on_eof_not_just_done`:
1. Spawn the agent with a temp `RUXEL_STATE_DIR`.
2. Send `Hello`, then a **cacheable** one-task `Plan` (e.g. a `copy` with
   `content:` writing to a scratch file under the temp dir — pick a module whose
   `probe_for` returns `Some`; `copy` writing a small file works).
3. Read the `TaskResult`.
4. Close the agent's **stdin** (drop the pipe) to simulate connection loss —
   do **not** send `Done`.
5. Wait for the agent to exit.
6. Assert `<RUXEL_STATE_DIR>/ledger/ledger.json` **exists and is non-empty**
   (the fingerprint was flushed on EOF).

Read the existing `protocol.rs` tests first (they show the framing helpers and
how `RUXEL_STATE_DIR`/spawn are set up) and mirror them. If sending a full
`Plan` is more than the existing helpers support, extend them minimally (this
overlaps with plan 017 — coordinate: a Plan-executing protocol test is also
017's Step; if 017 landed first, reuse its helper).

**Verify**: `cargo nextest run -p ruxel-agent protocol` → the new test passes;
the pre-existing protocol tests still pass.

### Step 3: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy -p ruxel-agent
--all-targets -- -D warnings` → 0; `cargo nextest run` → green.

## Test plan

- New: `ledger_flushes_on_eof_not_just_done` in `protocol.rs` (Plan→result→drop
  stdin→assert `ledger.json` written). This is the regression guard for §8.
- Optionally a `main.rs`-level note is unnecessary; the protocol test exercises
  the real exit path.

## Done criteria

ALL must hold:

- [ ] The clean-EOF branch flushes the ledger before returning
- [ ] The frame-error branch flushes before returning 64 (completed-task fingerprints durable)
- [ ] The panic hook is unchanged (still no flush)
- [ ] `protocol.rs` proves a ledger written on EOF-without-Done
- [ ] `cargo nextest run` green; clippy/fmt clean
- [ ] `plans/README.md` row for 007 updated

## STOP conditions

Stop and report if:
- The protocol test harness can't send a `Plan` without substantial new
  scaffolding and plan 017 hasn't landed — implement the minimal helper, or if
  that balloons, land Step 1 alone (the fix) with a `// TODO(plan 017): EOF
  flush regression test` and report that the test is deferred.
- Flushing on the frame-error path surfaces a corrupt-write concern you didn't
  expect — `flush` is atomic (tmp+rename), so it shouldn't; if it does, report
  the exact failure.

## Maintenance notes

- Controller-side Ctrl-C handling (graceful shutdown message vs. hard drop) is a
  separate improvement; the agent flushing on EOF makes the current
  `kill_on_drop(true)` behavior safe for the ledger. Note for the operator that
  a clean `Done` is still preferred (it's the normal path).
- Reviewer: confirm the panic path still does **not** flush — flushing
  post-panic could persist a half-updated in-memory map.

# Plan 017: Protocol integration + frame edge-case tests

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. Tests only;
> no runtime behavior change (unless a test surfaces a real bug — then STOP and
> report, don't silently fix).
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-proto/src/frame.rs crates/ruxel-agent/tests/protocol.rs crates/ruxel-agent/src/main.rs`
> If any changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none (but the Plan-executing test overlaps plan 007's Step 2 —
  coordinate; share one helper)
- **Category**: tests
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

The agent's **task-execution + ledger** path — everything that runs on the six
production hosts after handshake — is **never exercised off-VM**: the protocol
test spawns the real agent but only sends `Hello` + `Done`, so `execute_task`,
module dispatch, the check-mode command/shell skip, and the ledger fast-path/
record are all unreached (a regression there passes CI). Separately, `frame.rs`
tests cover clean roundtrip, truncated *body*, and oversize, but **not** the
mid-varint EOF branch, the varint-overflow guard, or the `Interrupted` retry —
the exact branches a mid-run disconnect exercises. This plan closes both gaps
with hermetic tests (local subprocess over pipes; no network, no VM).

## Current state

**Protocol test (`crates/ruxel-agent/tests/protocol.rs`):** spawns the agent
binary, manages `RUXEL_STATE_DIR`, and sends `Hello`+`Done` only (per the
auditor: it never sends a `Plan`). `execute_task` (`crates/ruxel-agent/src/main.rs:185-296`),
module dispatch (`modules/mod.rs:96-139`), the check-mode command/shell skip
(`main.rs:243-258`), the ledger fast-path (`main.rs:232-239`), and
`ledger.record` (`main.rs:277-285`) are unreached. The agent reads frames from
stdin and writes `Event` frames to stdout; the test already has framing helpers
(read them).

**Frame edge cases (`crates/ruxel-proto/src/frame.rs`):** `read_frame`
(`:23-65`) has:
- `:30` `Ok(0) if first_byte => return Ok(None)` (clean EOF at boundary — tested)
- `:31-36` `Ok(0) => Err(UnexpectedEof "EOF inside frame length")` — the
  **mid-varint EOF** branch, **untested** (the existing truncation test cuts the
  *body*, not the length prefix).
- `:38` `Err(Interrupted) => continue` — the retry, **untested**.
- `:47-52` `shift >= 64 => Err(InvalidData "frame length varint overflow")` —
  **untested**.
- `:54-59` oversize — tested (`oversized_frame_is_rejected`, `:118`).
The existing tests are in `frame.rs:67-132` (`roundtrip_through_buffer`,
`truncated_body_is_an_error`, `oversized_frame_is_rejected`) — mirror their
style (build byte buffers, feed a `&[u8]` reader).

**Convention**: `frame.rs` tests use `std::io` readers over `&[u8]`. For the
`Interrupted` case, implement a tiny custom `Read` that returns
`ErrorKind::Interrupted` once then real bytes. Protocol tests use
`std::process::Command` to spawn the agent binary (find the binary path via
`env!("CARGO_BIN_EXE_ruxel-agent")` or the pattern the existing test uses).

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Frame tests | `cargo nextest run -p ruxel-proto frame` | pass (incl. new) |
| Protocol tests | `cargo nextest run -p ruxel-agent protocol` | pass (incl. new) |
| Build | `cargo build --workspace` | exit 0 |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel-proto/src/frame.rs` — add edge-case tests to the existing
  `#[cfg(test)] mod tests`.
- `crates/ruxel-agent/tests/protocol.rs` — add a Plan-executing test and a
  ledger-replay test; extend the framing helper minimally if needed.

**Out of scope**:
- Changing `frame.rs`/`main.rs` behavior. If a test reveals a real bug (e.g. a
  varint edge mis-decodes), **STOP and report** — the fix is a separate change.
- The controller-side transport framing (`transport.rs`) — its own tests are
  plan 018's concern (varint codec unification).

## Git workflow

- Branch: `advisor/017-protocol-tests`
- Commit: `test(proto,agent): frame edge cases + agent Plan/ledger integration`
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Frame edge-case tests

Add to `frame.rs` tests:
- `mid_varint_eof_is_error`: feed a buffer that is a single byte with the
  high-bit set (0x80, "more bytes follow") then EOF; `read_frame` → `Err` with
  `ErrorKind::UnexpectedEof`.
- `varint_overflow_is_error`: feed ten `0x80` bytes (continuation forever);
  `read_frame` → `Err(InvalidData)` ("varint overflow") before allocating.
- `interrupted_read_retries`: a custom `Read` yielding `ErrorKind::Interrupted`
  on the first `read` then the bytes of a valid frame; `read_frame` → `Ok(Some(_))`
  (the retry path is exercised, no error).

**Verify**: `cargo nextest run -p ruxel-proto frame` → all new tests pass.

### Step 2: Agent executes a Plan (dispatch + result)

In `protocol.rs`, add `plan_executes_and_returns_result`:
1. Spawn the agent with a temp `RUXEL_STATE_DIR`.
2. Send `Hello`, read `HelloAck`.
3. Send a one-task `Plan` for a **cacheable, side-effect-local** module — e.g. a
   `copy` with `content:` writing to a scratch file under the temp dir, or a
   `command` running `echo` (command is not cacheable but proves dispatch). Use
   `copy` if you also want to exercise the ledger in Step 3.
4. Read the `TaskStart` + `TaskResult`; assert the status/changed and that the
   scratch file was written.
5. Send `Done`, assert clean exit 0.

Read the existing `protocol.rs` framing helpers first and reuse them; extend
minimally to build a `Plan{RenderedTask{iterations:[Iteration{...}]}}`.

**Verify**: `cargo nextest run -p ruxel-agent protocol` → the new test passes.

### Step 3: Ledger replay on a second identical run

Add `ledger_replays_converged_task`:
1. As Step 2, send a cacheable `copy` Plan; read the `changed:true` result;
   send `Done` (agent flushes ledger). (If plan 007 landed, EOF also flushes;
   either is fine — use `Done` for determinism.)
2. Spawn a **second** agent with the **same** `RUXEL_STATE_DIR`; send `Hello`
   then the **same** Plan (same `ledger_key`).
3. Assert the second `TaskResult` is `changed:false` **and** that it came from
   the cache (the scratch file already matches). Optionally assert timing/marker
   distinguishing cached replay from a real re-check — at minimum assert
   `changed:false`.

Add a `bad_params_task_fails_gracefully`: send a `Plan` whose `params_json` is
malformed; assert a `failed` `TaskResult` (exercises `main.rs:211-223`) and the
agent stays alive for the next frame.

**Verify**: `cargo nextest run -p ruxel-agent protocol` → both pass.

### Step 4: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- `frame.rs`: mid-varint EOF, varint overflow, interrupted-retry.
- `protocol.rs`: Plan-executes, ledger-replays-converged, bad-params-fails.
All hermetic (byte buffers; local subprocess + temp state dir; no network/VM).
The Plan-executing helper is shared with plan 007's flush test — if 007 landed
first, reuse its helper; if this lands first, 007 reuses yours.

## Done criteria

ALL must hold:

- [ ] `frame.rs` tests cover mid-varint EOF, varint overflow, and interrupted-retry
- [ ] `protocol.rs` proves the agent executes a `Plan` (dispatch + `TaskResult`)
- [ ] `protocol.rs` proves ledger replay: a second identical run returns `changed:false` from cache
- [ ] `protocol.rs` proves malformed params yield a `failed` result without killing the agent
- [ ] No behavior change to `frame.rs`/`main.rs` (or a real bug was found and reported, not silently patched)
- [ ] `cargo nextest run` green; clippy/fmt clean
- [ ] `plans/README.md` row for 017 updated

## STOP conditions

Stop and report if:
- A frame edge-case test reveals a real decode bug (e.g. overflow not caught) —
  that's a genuine finding; report it as a fix candidate, don't fold the fix
  into this tests-only plan.
- Building a `Plan` in the test requires exposing a lot of proto internals — the
  `ruxel_proto::v1` types are already public (they're the wire contract); use
  them directly.
- The ledger-replay test is flaky due to timing — assert on `changed:false` +
  file state, not on wall-clock.

## Maintenance notes

- These tests are the foundation for plan 024's chaos harness (inject
  disconnects at each protocol state). Keep the framing helpers reusable.
- Reviewer: the ledger-replay test is the one that would catch a future
  false-positive regression in the probe schema (plan 006) end-to-end — verify
  it actually asserts a cache *hit*, not just `changed:false` (which a real
  re-check would also produce). Assert the file is untouched between runs.

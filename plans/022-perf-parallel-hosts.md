# Plan 022: Parallelize hosts + fix the multi-host transport stall

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. This work
> is **partly diagnostic** (the transport stall is documented but not
> root-caused) — expect to reproduce before you fix.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel/src/transport.rs crates/ruxel/src/commands/apply.rs`
> If either changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED (touches the core transport every run depends on; a regression
  hits every gate)
- **Depends on**: 021 (best sequenced after the perf work; both touch
  transport/agent adjacency)
- **Category**: perf
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Two coupled facts block fleet-wide runs and the last M5 benchmark:
1. **Documented transport stall (DIRECTION-03)**: "the SECOND sequential connect
   inside one process stalls at the agent handshake"
   (`transport.rs:8-15` header). `apply.rs`'s host loop opens each host's
   connection **in the same process**, so any run without `--limit` against ≥2
   hosts hits the stall on host #2 today. Four workload playbooks target `all`
   (6 hosts). Real runs currently work only one-connect-per-process.
2. **Hosts run strictly sequentially (PERF-06)**: even with the stall fixed,
   `apply.rs:124-193` awaits each host to completion before the next — the
   ARCHITECTURE §1 "hosts run in parallel" promise is absent from the code
   shape. For the 6-host M5 target this is a 6× wall-clock difference.

**Honest scoping**: the M6 production pilot is *per-host* (`PLAN.md:149`) and the
common invocation is `--limit <one host>` (works today), so this is an
**M5-proof + fleet-ergonomics** improvement, **not** a pilot blocker. Sequence
it after the correctness (P1) and single-host perf work.

## Current state

- **Stall**: `transport.rs:8-15` header documents it (fixture-reproduced, also
  present with the old openssh crate — points at ControlMaster/stdio ownership
  under concurrent/sequential sessions in one process). Not root-caused.
- **Sequential hosts**: `apply.rs:124-193`:
  ```rust
  for host in hosts {
      let (mut conn, ack) = connect_with(&dest, agent_bin, run_id, false, &options).await?;
      let recap = run_play(play, &host.name, &ack.facts, engine, &mut conn, ...).await?;
      conn.shutdown().await?;
      // print recap; track any_failed
  }
  ```
  Awaits each host fully before the next; no `join_all`/task spawning.
- **Socket name**: `transport.rs:54-61` uses `ruxel-mux-{pid}-{subsec_nanos:x}` —
  two concurrent connects in the same nanosecond could collide on the socket
  name (a multi-host-parallelism hazard flagged in the correctness audit).
- **Master lifecycle**: `Master` (`transport.rs:43-50`) is a foreground `ssh -M
  -N` child; `AgentConnection` uses `kill_on_drop(true)`.
- Gate driver runs each connect in its own process today
  (`tools/fixtures/gate.sh`) precisely to avoid the stall.

**Convention**: async (`tokio`, `rt-multi-thread` is enabled). Fixtures are
disposable Hetzner VMs; you need **two** to test multi-host — operator
pre-approved fixture VMs (see `RESTORE.md`), never production. Interleaved output
needs per-host buffering.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Build | `cargo build -p ruxel-cli` | exit 0 |
| Transport gate (1 host) | `RUXEL_TEST_SSH_DEST=... cargo test ... transport_gate -- --ignored` | operator/fixture |
| Multi-host repro | two fixture VMs; `ruxel apply` (no `--limit`) against both | reproduces stall today |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel/src/transport.rs` — root-cause + fix the 2nd-connect stall;
  ensure per-host socket/master isolation; unique socket names.
- `crates/ruxel/src/commands/apply.rs` — spawn one task per host, join, aggregate
  recaps, serialize output.

**Out of scope**:
- Any host contact outside operator-provided fixtures (never production).
- The scheduler/pipelining internals (plan 020) — this is about the *host* loop,
  not the task loop.
- Warm-daemon tier.

## Git workflow

- Branch: `advisor/022-parallel-hosts`
- Commit: (1) transport stall fix; (2) per-host parallelism. Land the fix first.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Reproduce the 2nd-connect stall on two fixtures

**Safety**: use only operator-provided disposable fixture VMs; confirm each
target IP is not one of the six production hosts before any contact (record
`Safety check: target` per `GOAL.md` rule 2). Create two fixtures
(`tools/fixtures/create.sh`), then run a single-process `ruxel apply` (or a
minimal harness) that opens a connection to host A then host B sequentially, and
confirm B stalls at handshake. Capture where it hangs (add temporary tracing
around `connect_with`/`establish`/the HelloAck wait).

**Verify**: you can reproduce the stall deterministically and have identified
the hang point (likely shared ControlMaster socket/stdio ownership, or a global
resource the first `Master` holds). Do not proceed to a fix until the cause is
understood.

### Step 2: Fix the transport so concurrent per-host sessions are independent

Based on the root cause, give each host **fully independent** transport state:
- A **unique** mux socket per host+connection (add host identity + more entropy
  to the `transport.rs:54-61` name so two concurrent connects can't collide).
- Ensure each `Master`/`AgentConnection` owns its own child process and pipes
  with no shared global (the header hints the first session's stdin ownership
  interferes with the second — verify and isolate).
- If the issue is `ssh` ControlMaster reuse, consider one master per host (each
  its own `-S <socket>`), or drop ControlMaster in favor of a single direct
  `ssh` per host if that's cleaner for the 1-connection-per-host model.

**Verify**: the single-process two-host sequential connect from Step 1 now
completes both handshakes. The **existing single-host** transport gate still
passes (no regression). Operator/fixture-run.

### Step 3: Run hosts concurrently in `apply`

Replace the `for host in hosts` loop (`apply.rs:124-193`) with `join_all` (or
`tokio::spawn` + join) over per-host async fns, each owning its connection and
producing a `Recap`. Aggregate: sum recaps, `any_failed` if any host failed,
exit code per the contract (1 if any host failed). **Serialize stdout**: buffer
each host's human/JSON output and print per-host blocks so interleaving doesn't
scramble the recap (a `Mutex<Stdout>` or per-host buffers flushed in order).
Respect a sane concurrency cap (all workload fleets are ≤6 hosts; no cap needed,
but don't spawn unbounded if a future inventory is large).

**Verify**: a two-fixture `ruxel apply` (no `--limit`) completes both hosts
concurrently (wall-clock ≈ max(host times), not sum), output is not interleaved,
and the recap/exit code are correct. Single-host runs are unchanged.

### Step 4: Full gates + benchmark

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green. Operator captures the 6-host
parallel benchmark (the last M5 headline number) and commits it to
`docs/benchmarks/`.

## Test plan

- Unit: socket-name uniqueness (two calls produce different names), recap
  aggregation logic (sum + any_failed), output-ordering helper.
- Integration (operator/fixture, two VMs): the 2nd-connect stall is gone;
  concurrent hosts finish in ≈max time; output not scrambled; exit code correct.
- The single-host transport gate must stay green (no regression).

## Done criteria

ALL must hold:

- [x] Two hosts in one process both handshake; the historical stall did not reproduce in repeated sequential or concurrent fixture runs, so no speculative transport fix was made
- [x] Each host has an independent mux socket/master; socket names can't collide under concurrency
- [x] `apply` runs hosts concurrently; wall-clock ≈ max(host), not sum; output per-host, not interleaved
- [x] Recap aggregation + exit code correct (1 if any host failed)
- [x] Single-host transport gate unregressed; disposable 6-host benchmark captured
- [x] `cargo nextest run` green; clippy/fmt clean
- [x] `plans/README.md` row for 022 updated; `transport.rs` known-issue header removed/updated
- [x] No production host was ever contacted (only operator fixtures; `Safety check: target` recorded)

2026-07-16 diagnostic: two local disposable Alpine SSH containers, bound only
to `127.0.0.1:22221` and `127.0.0.1:22222`, both completed sequential
`connect_with` handshakes in one process in 0.84 s. This does **not** reproduce
the documented Hetzner fixture stall, so no speculative transport or host-loop
change was shipped. Both containers and their temporary key were destroyed.
The required two operator fixture targets remain unavailable.

Later on 2026-07-16, two labeled Hetzner fixtures were created in the isolated
project. A single controller process connected to both sequentially and both
completed; the stall still did not reproduce. Both fixtures were reaped. The
STOP condition therefore remains active only for speculative transport
rewrites. Bounded host orchestration was independently implemented and measured
on two new labeled fixtures: 1.31 s and 1.54 s individually, 1.42 s together.
Output remained grouped in inventory order and both recaps completed. Evidence:
`docs/benchmarks/multihost-parallel.md`.

The final six-host acceptance uses six isolated Debian SSH containers because
the provider safety cap is two. It measures isolated-host, serial-sum, Ruxel,
and pinned-Ansible timings; checks inventory-ordered recaps, structured
unreachable output, and repeated-run SSH process cleanup. Evidence and
reproduction command: `docs/benchmarks/six-host-local.md`.

## STOP conditions

Stop and report if:
- You cannot reproduce or root-cause the stall within a reasonable effort —
  **do not** ship a speculative fix to the core transport; report the hang point
  and diagnostics and stop.
- The fix requires abandoning ControlMaster (a larger architectural shift) —
  report the tradeoff (it changes the "operator's ssh config behaves identically"
  property) before proceeding.
- Any target resolves to a production IP — **STOP immediately** (safety rule),
  destroy fixtures, report.
- Two fixtures aren't available/authorized — the multi-host work can't be
  verified; land nothing and report (single-host is unaffected).

## Maintenance notes

- Reviewer: the transport is load-bearing for every run — insist on the
  single-host gate staying green and the two-host fix being understood, not
  guessed.
- After this, ARCHITECTURE §1 "hosts run in parallel" is true; update plan 002's
  doc notes and remove the `transport.rs` known-issue header.
- The socket-path-privacy change (plan 015 Step 1) and this socket-uniqueness
  change touch the same code — coordinate so both land coherently.

# Plan 020: Make `apply` consume the compiler (pipelining + enum revalidation)

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. This is a
> **large architectural change** — land it incrementally, behavior-preserving at
> each step, guarded by plan 009's scheduler tests.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel/src/scheduler.rs crates/ruxel/src/commands/apply.rs crates/ruxel-core/src/compiler.rs crates/ruxel-proto/proto/ruxel.proto`
> If any changed, re-verify excerpts; on mismatch, STOP.
> Confirm plan 009 (scheduler test seam) is merged.

## Status

- **Priority**: P2
- **Effort**: L (multi-day)
- **Risk**: MED (reorders/overlaps dispatch; must preserve playbook-order,
  notify, and failure-stop semantics — guarded by 009's tests + goldens + on-VM
  gate)
- **Depends on**: 009 (test seam)
- **Category**: perf / tech-debt
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Two problems share one root cause: **`apply` never uses the compiler.**
`compiler::compile` is called **only** from `plan.rs` (offline preview); the
scheduler re-renders each task just-in-time and sends **one task per `Plan`**,
then blocks on that task's `TaskResult` before the next (`scheduler.rs` has a
single `.send` site). Consequences:
1. **Perf (PERF-01)**: wall-clock grows linearly as (tasks + loop-items) ×
   SSH-mux RTT, fully serialized. For a 65-task setup-* playbook with per-item
   loops (100+ dispatches) at 15–40 ms RTT, that's ~1.5–4 s of pure round-trip
   stall that ARCHITECTURE §4's issue-window pipelining is designed to overlap
   down to a handful of true-data-dependency RTTs. (Invisible in the 8-task,
   loop-free, localhost benchmark — which is why it hasn't surfaced.)
2. **Correctness (DEBT-01)**: the compiler re-validates rendered literal-enum
   params against the closed surface (`validate_rendered_enums`,
   `compiler.rs:314-338`) at plan time — but `apply` renders independently and
   has **no** central equivalent, so a templated `state:`/`fstype:` that renders
   outside the surface is caught by `ruxel plan` but not centrally by `apply`.

The compiler already has the full `Readiness::Static`/`Deferred{waits_on}` DAG
(`compiler.rs:60-72`) and register read-set annotation — built and tested, just
unused by `apply`. The agent already accepts `PlanPatch` (`main.rs:149-150`).
This plan wires the existing pieces together.

## Current state

- **Compiler (unused by apply):** `compiler::compile(playbook, engine) -> Plan`
  (`compiler.rs:90`), with `PlanTask{ body, reads, provides, no_log }`
  (`:36-45`), `Readiness::Static{ params, free_form, loop_items }` /
  `Readiness::Deferred{ waits_on }` (`:60-72`), and `validate_rendered_enums`
  (`:314-338`). Only `plan.rs:54` calls it.
- **Scheduler (the naive path):** `run_module_task`/`execute_iterations`/
  `execute_once` (`scheduler.rs:221-545`) render JIT and, per iteration, build a
  one-task `Plan` (`:583-605`) and block on `next_event()` (`:607-629`). One
  `.send` in the whole file (`:584`). The header comment (`:6-8`) says pipelining
  "replace[s] this walk once the ledger lands" — the ledger landed; the walk did
  not get replaced.
- **Proto (ready):** `Plan{ tasks:[RenderedTask], blobs_referenced }` (`:39-42`),
  `PlanPatch{ tasks }` (`:45-47`); agent handles both identically
  (`main.rs:149-160`). `ProbeResult` does **not** exist (ARCHITECTURE §6's probe
  event — out of scope here; this plan pipelines execution, not a separate probe
  phase).
- **apply (renders via scheduler):** `apply.rs:150-165` calls `run_play`, which
  owns the JIT rendering. The compiler is not in this path.

**Convention**: async (`tokio`), `anyhow`. Observable semantics (recap counts,
notify, failure-stop, register shapes) are pinned by goldens and plan 009's
tests — those are the CONTROL for "behavior preserved."

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Scheduler tests | `cargo nextest run -p ruxel-cli scheduler` | pass (behavior preserved) |
| Full suite | `cargo nextest run` | no regressions |
| Build | `cargo build --workspace` | exit 0 |
| On-VM gate | `tools/fixtures/bless-gate.sh ...` | operator/fixture: converged rerun changed=0 |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel/src/scheduler.rs` — consume `compiler::Plan`; batch consecutive
  `Static` tasks into one wire `Plan`; render `Deferred` nodes from arriving
  registers and send `PlanPatch`; add the concurrent-receive side.
- `crates/ruxel/src/commands/apply.rs` — call `compiler::compile` and pass the
  compiled plan into the scheduler.
- `crates/ruxel-core/src/compiler.rs` — only if the apply path needs an
  additional accessor (keep changes minimal; the DAG exists).

**Out of scope**:
- The separate `ProbeResult` probe-phase event (ARCHITECTURE §6) — not built;
  this plan pipelines *execution*, keeping the ledger fast-path on the agent as
  is.
- The content-addressed blob channel (still inline `content:` shipping) — plan
  021/future.
- Per-host parallelism — plan 022.
- Changing any observable semantics.

## Git workflow

- Branch: `advisor/020-apply-compiler`
- Commit incrementally: (1) apply-time enum revalidation (small, standalone
  correctness win); (2) consume compiler DAG read-only; (3) batch static runs;
  (4) deferred + PlanPatch. Each commit behavior-preserving + tests green.
- Do NOT push/PR unless instructed.

## Steps

### Step 1 (standalone correctness): apply-time enum re-validation

Before the larger refactor, close DEBT-01's correctness gap cheaply: in the
`apply` render path (`scheduler.rs::execute_once`, after params are rendered at
`:501-505`), call the same closed-surface enum check the compiler uses. Either
reuse `compiler::validate_rendered_enums` (make it `pub` if needed) or factor it
into a shared helper both call. A templated `state:`/`fstype:`/etc. that renders
outside the surface must now hard-error at apply too, matching `plan`.

**Verify**: add a scheduler test (on plan 009's seam) where a task's `state:` is
a template that renders to an out-of-surface value; assert `apply` errors (not
silently proceeds). `cargo nextest run -p ruxel-cli scheduler` → pass. This
step is independently shippable.

### Step 2: Consume the compiled plan (read-only, no batching yet)

Have `apply.rs` call `compiler::compile(playbook, engine)` and pass the `Plan`
to `run_play`. Initially, keep the scheduler's execution loop but drive it from
the compiled `PlanTask` list (using each task's precomputed `Static` params when
available, falling back to JIT render for `Deferred`). This is a refactor with
**no behavior change** — every task still executes in order, one per wire `Plan`.
The point is to make the compiler the single source of rendering.

**Verify**: `cargo nextest run` → all existing scheduler/golden tests pass
(behavior identical). The on-VM bless-gate (operator) for one playbook still
shows converged rerun `changed=0`.

### Step 3: Batch maximal runs of `Static` tasks into one wire `Plan`

Group consecutive `Readiness::Static` tasks (with no intervening `Deferred` node,
`when` on a register, or controller-side module) into a **single** `Plan`
message with multiple `RenderedTask`s. The agent already loops over
`plan.tasks` (`main.rs:151-160`), so it drains them without per-task controller
stalls. The controller reads the stream of `TaskStart`/`TaskResult` events and
matches them by `task_id`. Preserve: playbook order in the batch, per-task
`notify`/register/recap, and **failure-stop** (on a `failed` `TaskResult` for a
non-ignored task, stop sending/consuming subsequent tasks for that host — you
may need to send a "stop" or simply stop reading and shut down; match today's
semantics exactly).

**Verify**: scheduler tests still pass; add a test that a batch of 3 static
tasks produces 3 `TaskResult`s in order with correct recap; a failing 2nd task
stops the 3rd (failure-stop preserved). `cargo nextest run -p ruxel-cli
scheduler` → pass. On-VM gate unchanged.

### Step 4: Render `Deferred` nodes from registers and stream `PlanPatch`

For `Readiness::Deferred{ waits_on }` tasks, render them **as their register
inputs arrive**: after the `TaskResult` that provides a name in `waits_on`,
render the deferred task and send it as a `PlanPatch` (the agent handles it
identically). The added latency is one controller round-trip **per true data
dependency edge** — not per task. Keep the register-dependency ordering correct
(a deferred task must not be sent before all its `waits_on` registers exist).

Implement the concurrent-receive side: the controller sends batched
static/patched tasks and consumes the event stream, rendering the next
window while the agent executes the current one.

**Verify**: a scheduler test with a stat→register→when-on-register→loop chain
executes correctly and the deferred task sees the register value; recap matches
the sequential baseline. `cargo nextest run` → green. **On-VM gate is the real
proof**: a converged 65-task-style playbook rerun must still be `changed=0` and
status-identical to Ansible, and measurably faster than the pre-pipeline
baseline (operator measures).

### Step 5: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- Step 1: apply-time enum-revalidation test (out-of-surface template errors).
- Step 3: batch-order + failure-stop tests (on 009's `FakeAgent`).
- Step 4: register-dependency chain test (deferred task sees register).
- All existing goldens/scheduler tests must stay green (behavior preservation is
  the primary gate). The **on-VM bless-gate** (operator) confirms real-world
  parity + the perf win.

## Done criteria

ALL must hold:

- [x] `apply` calls `compiler::compile` and drives execution from the compiled plan
- [x] Rendered enum params are re-validated at apply time (out-of-surface template → hard error, matching `plan`)
- [x] Consecutive `Static` tasks batch into one wire `Plan`; the agent drains them without per-task controller stalls
- [x] `Deferred` tasks render from arriving registers and stream as `PlanPatch`
- [x] Playbook order, notify, register shapes, and failure-stop are preserved (all 009 tests + goldens green)
- [x] Synthetic on-VM bless-gate: converged rerun status-identical to Ansible
- [x] `cargo nextest run` green; clippy/fmt clean
- [x] `plans/README.md` row for 020 updated

The invalid real-workload capture was removed. Replacement synthetic gates
passed on 2026-07-16 for ext4 storage, controller delegation, and PostgreSQL
ownership/default privileges. Ruxel and Ansible matched every converged task
status; deliberately always-changed/check-forced tasks retained their measured
Ansible changed behavior.

## STOP conditions

Stop and report if:
- Any golden or 009 scheduler test changes result — behavior drifted; the
  pipelining must be observably identical. Revert the offending step and report.
- Failure-stop semantics can't be preserved with batched tasks (the agent
  executes task 3 before the controller can react to task 2's failure) — this is
  the critical hazard; design the batch/stop protocol so a non-ignored failure
  halts the host **exactly** as today, and report your approach before shipping.
- The `--check`/`--diff`/`--tags`/no_log paths interact badly with batching —
  each must behave identically; if one doesn't, narrow batching to exclude that
  case and report.

## Maintenance notes

- This is the enabler for plan 021 (probe concurrency — the agent now has
  whole-batches to parallelize) and a prerequisite mindset for plan 022 (host
  parallelism). Keep the batch/PlanPatch protocol clean.
- Reviewer: the two non-negotiables are (a) observable behavior identical to the
  sequential path, and (b) failure-stop preserved. Everything else is
  latency optimization.
- After this lands, ARCHITECTURE §4 stops being "NOT YET BUILT" (plan 002's
  note) — update the doc.

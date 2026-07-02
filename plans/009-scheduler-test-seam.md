# Plan 009: Add a test seam to the scheduler control flow

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. This plan
> is a **refactor to enable testing** — observable behavior must not change.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel/src/scheduler.rs crates/ruxel/src/transport.rs crates/ruxel/src/lib.rs`
> If any changed, re-verify the excerpts; on mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED (touches the core apply path; guarded by existing goldens and
  the new tests it enables)
- **Depends on**: none (prerequisite for 010 and 020)
- **Category**: tests / tech-debt
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

`crates/ruxel/src/scheduler.rs` (758 lines) is the control-flow heart —
`when`/`loop`/`register`/`until`/`changed_when`/`failed_when`/block/rescue/
handlers/tags all live here — and it has **zero unit tests**. Its only
verification is the manual on-VM bless-gate over 6 of 16 playbooks. Reading it
(plan 010) already surfaced two real divergences from Ansible. The blocker to
testing it is that `run_play` requires a live `AgentConnection` (real SSH +
agent). This plan introduces a **module-execution trait boundary** so
`run_play` can run against an in-memory fake agent, making the pipeline branches
unit-testable. It changes no observable behavior; it unblocks plan 010's fixes
(which need to be verifiable) and plan 020's larger refactor.

## Current state

`crates/ruxel/src/scheduler.rs`:
- `HostRun<'a>` (`:38-54`) holds `conn: &'a mut AgentConnection` (the real
  transport) plus engine, scope vectors, recap, etc.
- `run_play(...)` (`:71-123`) builds a `HostRun` and walks
  `pre_tasks`/`tasks`/handlers via `run_task_or_block`.
- The **only** place the agent is invoked is `execute_once` (`:388-545`): for
  agent-side modules it renders params, then `self.conn.send(&Envelope{Plan{...}})`
  (`:583`) and loops on `self.conn.next_event()` (`:607`) awaiting the
  `TaskResult`, returning a `minijinja::Value` result dict. Controller-side
  modules (debug/set_fact/fail/assert/pause) return without touching `conn`
  (`:397-497`).
- `AgentConnection` is defined in `crates/ruxel/src/transport.rs`; its methods
  used by the scheduler are `send(&Envelope)` and `next_event() -> Result<Option<Event>>`.
- `crates/ruxel/src/lib.rs` re-exports `scheduler`, `transport`, etc. (the crate
  is both a bin `ruxel` and a lib `ruxel_cli`).

**The seam point**: everything the scheduler needs from the agent is "given a
rendered `Plan` (one task, one or more iterations), return the per-iteration
`TaskResult`(s)." That is the exact boundary to abstract.

**Convention**: async fns (`tokio`), `anyhow::Result`. The transport is already
behind a concrete struct; introduce a trait it implements. Tests use
`#[tokio::test]` — but note the crate's `tokio` features
(`crates/ruxel/Cargo.toml`) are `rt-multi-thread, macros, io-util, time,
process`; `macros` gives `#[tokio::test]`. Confirm before relying on it.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Build | `cargo build -p ruxel-cli` | exit 0 |
| Scheduler tests | `cargo nextest run -p ruxel-cli scheduler` | new tests pass |
| Full suite | `cargo nextest run` | no regressions |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel/src/scheduler.rs` — introduce a trait for the agent round-trip;
  make `HostRun` generic over it (or hold a `&mut dyn Trait`); add a
  `#[cfg(test)] mod tests` with a fake implementation.
- `crates/ruxel/src/transport.rs` — implement the new trait for
  `AgentConnection` (thin wrapper over existing `send`/`next_event`).

**Out of scope**:
- Any change to what the scheduler *computes* (behavior must be identical — this
  is the CONTROL: existing goldens and bless-gates must still pass).
- The register-on-skip / block-inheritance **fixes** — those are plan 010,
  layered on top of this seam.
- The compiler/pipelining refactor — plan 020.

## Git workflow

- Branch: `advisor/009-scheduler-seam`
- One commit: `refactor(scheduler): extract agent-execution trait for testing`
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Define the agent-execution trait

Define a trait capturing exactly the scheduler↔agent interaction. Minimal shape
— it must cover "send a one-task Plan and collect its TaskResult events." Two
viable granularities:
- **Coarse (recommended)**: a trait method that takes the rendered task/iteration
  data the scheduler assembles in `execute_once` (`:583-605`) and returns the
  parsed result `Value`(s). This keeps all the send/recv/`TaskResult`-parsing
  behind the trait, so the fake just returns canned results.
- Fine (send/recv passthrough): mirror `send`/`next_event`. More faithful but
  the fake must speak the event protocol.

Choose coarse. Define e.g.:
```rust
#[async_trait::async_trait]  // OR hand-roll with a boxed future if async_trait
                             // isn't a dep — check Cargo.toml first; prefer no new dep
pub(crate) trait AgentExec {
    async fn run_iteration(&mut self, req: &IterationRequest) -> anyhow::Result<minijinja::value::Value>;
}
```
where `IterationRequest` bundles what `execute_once` currently builds (module,
params_json, free_form, ledger_key, item_label, no_log, become_user,
environment, check_mode_override). **Do not add `async_trait` as a dependency
if it isn't already present** — if traits-with-async are awkward without it,
make the trait method return `Pin<Box<dyn Future>>` by hand, or make
`run_iteration` non-async and have the real impl block internally via the
existing connection (it already `.await`s inside `execute_once`). Simplest
path: keep the method `async` and make `HostRun` generic `<A: AgentExec>` so no
trait objects/boxing are needed and no new dep is required.

**Verify**: `cargo build -p ruxel-cli` → 0 after defining the trait (no callers
yet).

### Step 2: Move the agent round-trip in `execute_once` behind the trait

Extract the block in `execute_once` that assembles the `Plan` and awaits the
`TaskResult` (`:583-629`) into the trait's `run_iteration`. The real
implementation is a wrapper holding `&mut AgentConnection` that does exactly
what the code does today (send Plan, loop `next_event`, parse `TaskResult` into
the `Value`). `HostRun` holds `agent: A` (generic) instead of `conn: &mut
AgentConnection`; `run_play` is generic over `A` and `apply.rs` passes the real
wrapper around the `AgentConnection`.

Keep controller-side modules (debug/set_fact/etc.) exactly as they are — they
never touch the agent.

**Verify**: `cargo build -p ruxel-cli` → 0; `cargo nextest run` → **all existing
tests still pass** (this is the behavior-preservation check). If a bless-gate or
golden exists that runs here, it must be unchanged.

### Step 3: Update `apply.rs` to construct the real agent-exec wrapper

`crates/ruxel/src/commands/apply.rs:150-165` calls `run_play(play, &host.name,
&ack.facts, engine, &mut conn, ...)`. Change it to wrap `conn` in the real
`AgentExec` impl and pass that. The signature change is internal to `ruxel-cli`.

**Verify**: `cargo build -p ruxel-cli` → 0; `cargo clippy --all-targets -- -D
warnings` → 0.

### Step 4: Add a fake agent and the first characterization tests

In `#[cfg(test)] mod tests` in `scheduler.rs`, implement `AgentExec` for a
`FakeAgent` that returns scripted results keyed by call order or module (e.g. a
`VecDeque<Value>` of canned results, or a closure). Write **characterization
tests that pin current behavior** (these are the CONTROL for plan 010):
- a single `command` task → recap `ok=1 changed=1` (bare command always changed)
- a `when: false` single task → recap `skipped=1`
- a loop over 2 items, one item `when` false → recap reflects per-item skip
- a `changed_when: false` task → recap `ok=1 changed=0`
- a `block` with a failing task + `rescue` → recap `rescued=1`, host not failed
- a `notify` + handler: changed task notifies, handler runs at end of play

Build minimal `Play`/`Task` structs in-test (read `crates/ruxel-core/src/playbook.rs`
for the exact struct shapes and constructors, or build them via the parser from
a small YAML string — using the parser is more robust and mirrors real input).

**Verify**: `cargo nextest run -p ruxel-cli scheduler` → the new tests pass and
document today's behavior.

### Step 5: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green (no regressions anywhere).

## Test plan

- New `#[cfg(test)] mod tests` in `scheduler.rs` with a `FakeAgent` implementing
  `AgentExec`, driving `run_play` over small in-memory (or parser-built) plays.
- Cover the SEMANTICS §3 branches listed in Step 4. These are **characterization**
  tests (assert current behavior); plan 010 will add/flip the ones for the two
  known bugs.
- Prefer building plays via `ruxel_core::playbook::parse` from YAML string
  literals so the tests exercise the real parser→scheduler path.

## Done criteria

ALL must hold:

- [ ] An `AgentExec` (or equivalent) trait abstracts the scheduler↔agent round-trip; `AgentConnection` implements it
- [ ] `run_play` runs against a `FakeAgent` in unit tests with no SSH/agent
- [ ] ≥ 6 characterization tests pass, covering when/loop/changed_when/block-rescue/notify
- [ ] `cargo nextest run` shows **no regressions** in existing tests (behavior preserved)
- [ ] No new external dependency added (or, if `async_trait` is genuinely needed, it's justified in the PR)
- [ ] `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] `plans/README.md` row for 009 updated

## STOP conditions

Stop and report if:
- Making the trait async cleanly requires a new dependency (`async_trait`) —
  first try the generic-`HostRun<A>` approach (no boxing, no dep); only if that
  fails should you report and ask before adding a dep.
- Any existing golden/bless test changes result after the refactor — that means
  behavior drifted; revert and report (the seam must be behavior-neutral).
- The `Play`/`Task` structs can't be built in-test without exposing a lot of
  `ruxel-core` internals — switch to parser-built plays from YAML strings.

## Maintenance notes

- This seam is reused by plan 010 (fix tests) and plan 020 (pipelining
  refactor). Keep the trait minimal and stable.
- Reviewer: the critical property is behavior preservation — scrutinize that the
  extracted `run_iteration` does byte-identically what `execute_once` did
  (same Plan shape, same `TaskResult` parsing).
- The `FakeAgent` is the foundation for plan 024's chaos tests (inject
  error/timeout results per iteration).

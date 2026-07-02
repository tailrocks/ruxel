# Plan 010: Fix register-on-skip and block keyword/`always` inheritance

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel/src/scheduler.rs crates/ruxel-core/src/playbook.rs`
> If either changed, re-verify excerpts; on mismatch, STOP.
> Confirm plan 009 (test seam) is merged (`git log --oneline | grep -i "009\|seam"`); this plan's tests need it.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED (changes observable control-flow toward Ansible parity; guarded
  by 009's seam + new tests + on-VM bless-gate)
- **Depends on**: 009 (module-execution test seam)
- **Category**: bug (correctness)
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Reading the untested scheduler surfaced three real divergences from Ansible:

1. **CORRECTNESS-04 (register-on-skip)**: a `register`ed single-shot task that
   `when`-skips, and a `register`ed task whose `loop` is empty, **never bind
   their variable**. SEMANTICS §3.8: "Registered even on failure and on skip."
   Downstream `reg.skipped` / `reg is defined` / `reg is skipped` then error
   (`AnsibleUndefinedVariable`) or evaluate opposite to Ansible — and the
   stat→register→`when` idiom is pervasive in the workload, so a would-be-skipped
   host can become a hard failure.
2. **CORRECTNESS-05 (block keyword inheritance)**: the block arm threads only
   tags and evaluates the block `when`; it does **not** propagate the block's
   `become`/`become_user`/`vars`/`environment` to child tasks. SEMANTICS §4:
   "Keywords on the block (become, when, tags) are inherited." A block that
   groups tasks under `become_user: postgres` would run its children as **root**
   → wrong ownership / psql peer-auth as the wrong role.
3. **Block `always:` dropped**: the block arm destructures `always: _` and never
   runs it. The workload doesn't currently use `always` (SEMANTICS §4: "`always`
   not used"), so this is latent — but the parser and compiler accept `always`,
   so a future edit silently loses cleanup tasks. Fix it (small) and add a guard
   test.

## Current state

`crates/ruxel/src/scheduler.rs`:

**Register-on-skip** — `run_module_task` (`:221-300`):
- Single-shot `when` false (`:245-254`):
  ```rust
  if let Some(when) = &task.when {
      let outcomes = eval_when_parts(self.engine, when, &scope)?;
      if outcomes.iter().any(|ok| !ok) {
          let fc = task_eval::first_false_condition(when, &outcomes);
          let skip = task_eval::skipped_result(&fc);
          self.finish_task(task, out, skip, "skipped", false);
          return Ok(false);   // <-- returns BEFORE the register bind at :284
      }
  }
  ```
- Empty loop (`:258-263`): builds `task_eval::loop_aggregate(vec![])`,
  `finish_task(..., "skipped", ...)`, `return Ok(false)` — also before the bind.
- The register bind is at `:284-286`:
  ```rust
  if let Some(reg) = &task.register {
      self.register(reg, result.clone());
  }
  ```
- `finish_task` (`:634-673`) does **not** register (confirmed: it only updates
  recap + prints).
- `task_eval::skipped_result` (`crates/ruxel-core/src/task_eval.rs:14-27`) and
  `loop_aggregate(vec![])` (`:63-71`) already build the exact Ansible skip dict
  — they're just never bound in these two paths.

**Block inheritance** — `run_task_or_block` (`:163-219`):
```rust
if let TaskBody::Block { block, rescue, always: _ } = &task.body {
    let mut child_tags: Vec<String> = inherited_tags.to_vec();
    child_tags.extend(task.tags.iter().cloned());
    if let Some(when) = &task.when { /* block-level when gates the block */ }
    // ... runs `block`, then `rescue` on failure ...
    // NEVER runs `always`; NEVER propagates become/become_user/vars/environment
}
```
- Child tasks read only their own `task.become_user` when building the wire task
  (`:599`). There is no inherited-context threading.
- The `Task`/`TaskBody::Block` shapes are in `crates/ruxel-core/src/playbook.rs`
  (read it to see whether a block carries `become`/`become_user`/`vars`/
  `environment` fields — you must confirm which keywords a block node actually
  parses before propagating them).

**Convention**: scope layering is `Scope::with_layer(vec![(name, VarValue)])`
(see `:266-267`, `:339-343`). Inheritance should be a lower-precedence layer
than the task's own values.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Scheduler tests | `cargo nextest run -p ruxel-cli scheduler` | new/updated tests pass |
| Full suite | `cargo nextest run` | no regressions |
| Playbook struct check | `grep -n "struct Task\|enum TaskBody\|become_user\|environment\|pub vars" crates/ruxel-core/src/playbook.rs` | shows block's fields |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel/src/scheduler.rs` — the two early-return paths and the block arm.
- `crates/ruxel/src/scheduler.rs` `#[cfg(test)] mod tests` (from plan 009) — add
  the bug-fix tests.
- `crates/ruxel-core/src/playbook.rs` — **only if** a block node doesn't already
  carry the keywords needed to inherit (read first; change only if required, and
  keep it minimal).

**Out of scope**:
- The compiler-side block handling (`compiler.rs`) beyond what's needed for
  parity — plan 020 owns the compiler path.
- Reworking `become_user` semantics generally — only propagate block→child.
- `always` semantics beyond running it after block/rescue (no `always`-specific
  failure interaction is in the workload; implement the straightforward "always
  runs regardless of block/rescue outcome").

## Git workflow

- Branch: `advisor/010-scheduler-fixes`
- Commit per fix or one `fix(scheduler): register on skip, inherit block keywords + run always`.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Bind `register` on single-shot skip and empty loop

In `run_module_task`, before **each** early `return Ok(false)` in the skip
(`:251-252`) and empty-loop (`:261-262`) branches, bind the register if present:
```rust
if let Some(reg) = &task.register {
    self.register(reg, /* the skip/aggregate Value */ .clone());
}
```
Use the same `skip`/`agg` Value that `finish_task` already received, so the
registered variable is the exact Ansible skip dict (`skipped: true`, etc.).

**Verify**: add scheduler tests (using the 009 `FakeAgent`):
- `skipped_single_task_registers_skip_dict`: a `when: false` task with
  `register: r`; a following `debug`/`assert` reading `r.skipped` sees `true`
  (not undefined). Assert via a subsequent task whose `when: r is skipped`
  selects it, or inspect the registered scope directly if the test can.
- `empty_loop_registers_aggregate`: `loop: []` with `register: r`; `r.skipped`
  is `true` and `r.results` is `[]`.
`cargo nextest run -p ruxel-cli scheduler` → pass.

### Step 2: Propagate block keywords to child tasks

First **read `playbook.rs`** to confirm which keywords a `TaskBody::Block` node
parses (become/become_user/vars/environment/when/tags). For each that exists,
thread it into the children:
- Extend `run_task_or_block`'s block arm to build an inherited context (the
  block's `become_user`, `become`, merged `vars` as a scope layer, merged
  `environment`) and pass it down through the recursive `run_task_or_block`
  calls. A clean way: add an `inherited: &InheritedCtx` parameter (alongside
  `inherited_tags`) carrying `Option<become_user>`, `become`, and vars/env
  layers; children apply it with **lower precedence** than their own values
  (task's own `become_user` wins over the block's).
- At the point the wire task is built (`:599` `become_user: task.become_user...`),
  fall back to the inherited `become_user` when the task's own is empty.
- Merge inherited `environment` under the task's `environment` (task keys win),
  and push inherited `vars` as a lower scope layer in `scope()`.

If a block node does **not** currently parse `become_user`/`vars`/`environment`
(only `when`/`tags`), then the inheritance gap is smaller than feared — in that
case propagate whatever it does parse and note in the PR which keywords a block
can carry.

**Verify**: `become_user_inherited_from_block`: a block with `become_user:
someuser` containing a `command` task with no own `become_user`; assert the wire
`RenderedTask.become_user` the `FakeAgent` receives is `someuser` (the FakeAgent
can capture the `IterationRequest` from plan 009). `cargo nextest run -p
ruxel-cli scheduler` → pass.

### Step 3: Run block `always`

Change the block arm to bind `always` (not `always: _`) and run it **after** the
block/rescue sequence, regardless of whether the block failed or was rescued
(mirror Ansible: `always` runs in both success and failure paths). Preserve the
host-failed propagation: if the block failed and there is no rescue, `always`
still runs, then the host stops. Sequence:
1. run block; if a task fails → jump to rescue (existing).
2. run `always` (new) — always, after 1.
3. return the appropriate host-failed signal.

**Verify**: `block_always_runs_on_success` and `block_always_runs_after_rescue`
scheduler tests (a marker task in `always` executes in both paths). `cargo
nextest run -p ruxel-cli scheduler` → pass.

### Step 4: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

New scheduler tests (on plan 009's seam):
- register-on-skip (single + empty loop) — the CORRECTNESS-04 regression guard.
- block `become_user` inheritance — CORRECTNESS-05 guard.
- block `always` runs on success and after rescue — the latent-bug guard.
All use `FakeAgent`; build plays via `ruxel_core::playbook::parse` from YAML
string literals so the parser path is exercised.

## Done criteria

ALL must hold:

- [ ] A `when`-skipped single task and an empty-loop task both bind their `register` variable to the Ansible skip/aggregate dict
- [ ] Block `become_user` (and any other keyword a block node parses) is inherited by child tasks, with task-own values taking precedence
- [ ] Block `always` runs after block/rescue in both success and failure paths
- [ ] New scheduler tests cover all three and pass on the 009 seam
- [ ] `cargo nextest run` green; clippy/fmt clean; no existing golden regressed
- [ ] `plans/README.md` row for 010 updated

## STOP conditions

Stop and report if:
- `playbook.rs` shows a block node parses **no** become/vars/environment (only
  when/tags) — then CORRECTNESS-05's worst case (become_user under a block)
  can't occur through the parser; implement inheritance for what *is* parsed and
  report the reduced scope rather than inventing fields.
- Register-on-skip binding changes an existing golden/bless result — that golden
  may have encoded the bug; re-read it against Ansible semantics before
  overriding, and report.
- `always` semantics interact with rescue/host-failed in a way the tests can't
  pin cleanly — report your chosen ordering and reasoning.

## Maintenance notes

- After this lands, a converged bless-gate rerun for any playbook that uses
  blocks or register+when should still be `changed=0` and status-identical to
  Ansible — that's the real-world confirmation (operator/on-VM, not CI).
- Reviewer: the register-on-skip fix is the highest-impact — verify the *exact*
  dict shape bound matches `task_eval::skipped_result`/`loop_aggregate` (already
  pinned by goldens E-series).
- If plan 020 later moves rendering/dispatch, it must preserve these three
  behaviors — they now have tests, so 020's refactor will catch regressions.

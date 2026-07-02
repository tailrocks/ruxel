# Plan 001: Correct the stale "do not build the engine yet" docs

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving on. If
> anything in "STOP conditions" occurs, stop and report — do not improvise.
> When done, update this plan's status row in `plans/README.md`.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- AGENTS.md README.md CLAUDE.md`
> If either file changed since this plan was written, compare the "Current
> state" excerpts against the live text before proceeding; on a mismatch,
> treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: docs
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

`AGENTS.md` is the rules file every AI agent loads at session start (via
`CLAUDE.md`, which is just `@AGENTS.md`). It currently says the project is in
"Research and design" phase and **"Do not start building the execution engine
until the operator explicitly moves the project to implementation."** The
operator moved to implementation long ago — the engine is feature-complete (36
modules, ledger, full CLI; see `GOAL.md` and `RESTORE.md`, and the `feat(...)`
commits in `git log`). An agent that obeys `AGENTS.md` literally today will
**refuse to write engine code**, directly contradicting `GOAL.md`'s
operational contract. The README's front-page Status says the same thing
("the engine intentionally does not yet [exist]"), misdirecting every human
reader too. This is a two-file text edit with outsized leverage.

## Current state

- `AGENTS.md` — the agent rules file, loaded every session via `CLAUDE.md`.
  Lines 20–25 read:
  ```
  ## Project phase

  Research and design. Do not start building the execution engine until the
  operator explicitly moves the project to implementation. The current
  deliverables are the documents in `docs/` and the CLI skeleton.
  ```
  The surrounding sections — "Hard rule: never touch the production servers",
  "Scope discipline", "Conventions" — are **correct and must stay verbatim**.
- `README.md` — repo front page. Lines 16–19 read:
  ```
  ## Status

  Research and design phase. The CLI shape exists; the engine intentionally
  does not yet. Read the docs in order:
  ```
- `CLAUDE.md` — one line, `@AGENTS.md`. No change needed (it inherits 001's
  fix automatically). Do not edit it.
- Ground-truth state, to mirror in the new wording (from `RESTORE.md:13-25`):
  implementation is feature-complete (all 36 modules, convergence ledger, full
  `plan`/`apply` CLI, `op` secret resolver, SSH transport); 6 of 16 playbooks
  are gated three-way against real Ansible; milestones M0–M3 done, M4/M5
  partial, M6 (operator-driven production pilot) deliberately untouched.

- **Convention**: these are prose docs; match the existing terse, declarative
  tone. Keep the exact markdown heading levels already in each file.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Drift check | `git diff --stat b5f98ba..HEAD -- AGENTS.md README.md` | empty or reviewed |
| Confirm edits | `git diff AGENTS.md README.md` | shows only the intended text changes |
| Sanity: no code touched | `git status --porcelain` | only `AGENTS.md`, `README.md` modified |

## Scope

**In scope** (only files you may modify):
- `AGENTS.md` (the "## Project phase" section only)
- `README.md` (the "## Status" section only)

**Out of scope** (do NOT touch):
- `CLAUDE.md` — inherits the fix; editing it is unnecessary.
- The "Hard rule", "Scope discipline", "Conventions" sections of `AGENTS.md` —
  they are correct.
- `GOAL.md`, `RESTORE.md`, `docs/*` — reconciled in plans 002/003; not here.
- Any `crates/**` source file — this is a docs-only plan.

## Git workflow

- Branch: `advisor/001-docs-build-status`
- One commit; conventional-commit style (match `git log`, e.g.
  `docs: correct stale "research/design phase" status (engine is built)`).
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Rewrite the `AGENTS.md` "Project phase" section

Replace the three-line "Research and design…" paragraph under
`## Project phase` with wording that reflects active implementation and points
at the operational contract. Target shape (adjust prose, keep it terse):

```markdown
## Project phase

Implementation. The execution engine is built and feature-complete (all
closed-surface modules, the convergence ledger, the full `plan`/`apply` CLI,
and the `op` secret resolver). Current work is verification breadth (gating
the remaining playbooks) and the M5 performance/hardening pass; M6 is the
operator-driven production pilot and stays untouched by autonomous work.

`GOAL.md` is the active operational contract and `RESTORE.md` the latest
state snapshot — read both at session start. The design docs in `docs/`
remain normative for behavior.
```

Do not alter the production-safety rule above it.

**Verify**: `git diff AGENTS.md` → shows only this section changed; the
"never touch the production servers" text is untouched.

### Step 2: Rewrite the `README.md` "Status" section

Replace the "Research and design phase. The CLI shape exists; the engine
intentionally does not yet." sentence. Keep the "Read the docs in order:"
lead-in and the numbered doc list that follows (that list is correct). Target:

```markdown
## Status

Implementation is feature-complete: the closed-surface modules, the
convergence ledger, the full `plan`/`apply` CLI, and the `op`-backed secret
resolver are built and verified, with 6 of 16 workload playbooks gated
three-way against pinned Ansible. Remaining work is verification breadth and
the M5 performance proof. Read the docs in order:
```

**Verify**: `git diff README.md` → only the Status paragraph changed; the
numbered doc list below it is intact.

## Test plan

No code changes, so no automated tests. Verification is the two `git diff`
inspections above plus:

- `grep -n "does not yet\|Do not start building" AGENTS.md README.md` → **no
  matches** (the stale claims are gone).

## Done criteria

ALL must hold:

- [ ] `grep -rn "Do not start building the execution engine" AGENTS.md` → no matches
- [ ] `grep -rn "engine intentionally does not yet" README.md` → no matches
- [ ] `AGENTS.md` still contains the "never touch the production servers" hard rule (`grep -n "never" AGENTS.md` → matches present)
- [ ] `git status --porcelain` shows only `AGENTS.md` and `README.md` modified
- [ ] `plans/README.md` status row for 001 updated

## STOP conditions

Stop and report (do not improvise) if:

- The "Current state" excerpts don't match the live files (docs drifted since
  `b5f98ba`).
- You find a third file that also asserts the pre-implementation phase and are
  unsure whether it's in scope — report it (candidates are handled in plan 002;
  don't silently edit them here).

## Maintenance notes

- These status blurbs go stale every milestone. A durable fix (out of scope
  here) is to make the README Status link to `RESTORE.md` instead of restating
  the phase; leave that to the operator.
- Reviewer: confirm no behavioral doc (`docs/SEMANTICS.md`, `docs/ARCHITECTURE.md`)
  was edited here — those are plan 002's job and have different reconciliation
  rules.

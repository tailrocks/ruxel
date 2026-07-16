# Plan 023: Run log, `--detailed-exitcode`, and `--diff` for line-edit modules

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. **Security
> note**: the run log must honor `no_log` redaction (coordinate with plan 008) —
> never persist a secret.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel/src/scheduler.rs crates/ruxel/src/commands/apply.rs crates/ruxel-agent/src/modules/`
> If any changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: 008 (no_log redaction must be honored before persisting the
  run log)
- **Category**: direction (concrete build) — M6 pilot substrate
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Three small, self-contained features that make the cautious M6 production pilot
(run `ruxel plan`, compare, graduate host-by-host) practical — all promised in
`ARCHITECTURE.md §7` but absent:
1. **Run log** (`~/.local/state/ruxel/runs/<ts>-<run_id>.jsonl`, secrets
   redacted) — the forensic artifact answering "what did ruxel do on titan last
   Tuesday" during the pilot, and the substrate the deferred drift dashboard
   (§9) will need. The `--output json` event stream is already serialized;
   this tees it to a pruned file.
2. **`--detailed-exitcode`** (terraform-style: 0 = converged, 2 = changes
   applied/needed) — lets the operator *script* the pilot ("did anything
   change?") without scraping human output.
3. **`--diff` for lineinfile/replace/blockinfile** — `ruxel plan`'s core pilot
   value is diff-review; `copy`/`template` emit a real unified diff today but the
   23 line-editing tasks (fstab/pam/ssh config) show *that* a change would happen,
   not *what* line changes — exactly where the operator most wants to see it.

## Current state

- **Run log**: none. `grep -rn "state/ruxel\|runs/" crates/` → nothing.
  `--output json` (`apply.rs:108-112`, `scheduler::OutputFormat::Json`) is an
  ephemeral stdout stream (`scheduler.rs:688-699` emits one JSON object per task
  event), never persisted. `GOAL.md:376` lists the run log as a pending "Next
  rock."
- **`--detailed-exitcode`**: not in the clap surface (`ApplyArgs`
  `apply.rs:12-51` / `PlanArgs` `plan.rs:13-35`); grep for `detailed_exitcode` →
  nothing. The recap already tracks `changed` (`scheduler::Recap.changed`).
- **`--diff`**: `copy.rs:31-34` emits a real `unified_diff`; the helper is
  `modules/mod.rs:44-65` `unified_diff`. `apt.rs:86` emits empty `{}`;
  `lineinfile`/`replace`/`blockinfile` emit **no** `diff` field. The `diff_mode`
  plumbing (`ExecContext.diff_mode`, `modules/mod.rs:36-37`) already exists and
  each of these modules already computes before/after content internally.

**Convention**: content modules put `result["diff"] = unified_diff(before,
after)` when `ctx.diff_mode` (see `copy.rs:32-34`). The scheduler suppresses
diffs for `no_log` (`scheduler.rs:663-672`) — the run log must do the same.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Build | `cargo build --workspace` | exit 0 |
| Agent/CLI tests | `cargo nextest run` | pass |
| Manual diff | `ruxel apply --check --diff -i ... <playbook>` | line modules now show diffs |
| Manual exitcode | `ruxel apply --detailed-exitcode ...; echo $?` | 2 if changes, 0 if converged |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel-agent/src/modules/{lineinfile,replace,blockinfile}.rs` — emit
  `diff` under `diff_mode`.
- `crates/ruxel/src/scheduler.rs` and/or `commands/apply.rs` — the run-log writer.
- `crates/ruxel/src/commands/apply.rs` (+ `plan.rs`) — the `--detailed-exitcode`
  flag and exit mapping.

**Out of scope**:
- The warm-daemon drift dashboard (§9) — the run log is the substrate; don't
  build the dashboard.
- Changing the default exit-code contract (plan 013) — `--detailed-exitcode` is
  **opt-in** and layers on top; its "2" means "changes", distinct from plan 013's
  "2 = parse error" (which applies when the flag is absent).

## Git workflow

- Branch: `advisor/023-runlog-diff`
- Commit per feature (each independent).
- Do NOT push/PR unless instructed.

## Steps

### Step 1: `--diff` for lineinfile / replace / blockinfile

In each of `lineinfile.rs`, `replace.rs`, `blockinfile.rs`, when `ctx.diff_mode`
and the file changed, set `result["diff"] = super::unified_diff(&before, &after)`
using the before/after content each already builds (`replace.rs` has `current`
and `next`; `blockinfile.rs` has `current` and `next`; `lineinfile.rs` similarly).
Respect `no_log` — but note the agent-side `no_log` gating is plan 008's job; if
008 landed, the diff is already suppressed for no_log tasks via `ExecContext.no_log`;
if not, still emit the diff (the controller suppresses printing for no_log at
`scheduler.rs:663-672`) and add a `// coordinate with plan 008` comment.

**Verify**: unit tests — a `lineinfile`/`replace`/`blockinfile` change under
`diff_mode: true` produces a non-empty `diff` field showing the changed lines; no
`diff` when unchanged. `cargo nextest run -p ruxel-agent` → pass.

### Step 2: `--detailed-exitcode` flag

Add `--detailed-exitcode` (bool) to `ApplyArgs` (and `PlanArgs` if it applies to
plan). After the run, if the flag is set: exit **2** when any changes were
applied/needed (recap `changed > 0`), **0** when fully converged, **1** on host
failure (failure wins). When the flag is **absent**, keep today's behavior +
plan 013's contract (0/1/2-for-parse). Route through the `ExitCode` mechanism
(coordinate with plan 013; if 013 landed, extend its mapping; if not, use
`std::process::exit`).

**Verify**: manual (documented in PR): a converged run with the flag → 0; a run
that changes something → 2; a host failure → 1. Add a unit test for the recap→
exitcode mapping under the flag.

### Step 3: Run log

Tee the serialized JSON event stream to `~/.local/state/ruxel/runs/<ts>-<run_id>.jsonl`
(create the dir 0700; `<ts>` from the run start — note `Date::now()` is available
at runtime in the real binary, unlike in workflow scripts). Write the **same**
redacted events the `--output json` path emits (so `no_log` is already honored —
verify it is; if the JSON path leaks anything under no_log, fix that with plan
008's redaction). Prune old logs by count (keep the newest N, e.g. 50). The log
is **never** a dependency of execution — a write failure must not fail the run
(log a warning and continue). Write it regardless of `--output` (human or json).

**Verify**: after a run, `~/.local/state/ruxel/runs/` contains a `.jsonl` with
one event per line; a `no_log` task's payload is redacted in it (test with a
synthetic value); pruning keeps ≤ N files. `cargo nextest run` → green.

### Step 4: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- Diff: per-module non-empty diff under `diff_mode`, empty when unchanged.
- Exit code: recap→code mapping under `--detailed-exitcode` (0/2, 1 on failure).
- Run log: file written with redacted events; pruning by count; write-failure is
  non-fatal. Use synthetic non-secret values in all tests.

## Done criteria

ALL must hold:

- [x] `lineinfile`/`replace`/`blockinfile` emit a unified `diff` under `--diff`
- [x] `--detailed-exitcode` returns 2 on changes, 0 converged, 1 on failure; absent → today's contract
- [x] Every run writes a redacted `~/.local/state/ruxel/runs/<ts>-<run_id>.jsonl`, pruned by count, non-fatal on write error
- [x] `no_log` payloads are redacted in the run log (test proves it)
- [x] `cargo nextest run` green; clippy/fmt clean
- [x] `plans/README.md` row for 023 updated; ARCHITECTURE §7 items un-marked "unbuilt" (coordinate with plan 002)

## STOP conditions

Stop and report if:
- The `--output json` event stream is found to leak a secret under `no_log`
  (plan 008 should have fixed this) — do **not** persist the run log until that
  leak is closed; report and depend on 008.
- `--detailed-exitcode`'s "2" collides confusingly with plan 013's "2 = parse
  error" — they only coexist when the flag is set on a *successful* parse, so
  they don't actually conflict; if the mapping gets tangled, document the
  precedence explicitly (failure=1 > changes=2 > success=0, parse-error=2 only
  when unparsed).

## Maintenance notes

- The run log is the substrate for the deferred drift dashboard (ARCHITECTURE
  §9) — keep its schema stable (it's the `--output json` event schema).
- Reviewer: the security-critical property is redaction — a `no_log` secret must
  not land in the persisted log. Verify with a synthetic secret test.
- These three close the concrete half of ARCHITECTURE §7; the live-probe `plan`
  (as opposed to today's offline preview) is a larger item tied to plan 020.

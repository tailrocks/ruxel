# Plan 004: Fix CI — cache parity, honest fidelity gate, advisory/license gate

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- .github/workflows/ci.yml crates/ruxel-core/tests/workload.rs crates/ruxel-core/tests/render_parity.rs README.md`
> If any changed, re-verify excerpts before editing; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx / tests
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Three separate CI problems: (1) the `clippy` and `agent-cross` jobs have **no
`target/` cache and no sccache**, so both cold-build the entire dependency
graph on every push — the dominant CI wall-clock, avoidable. (2) The one test
command is **green while silently skipping the most important gates**: the
16-playbook compile gate and the 41-template byte-parity gate `return` early
(counting as *passed*) when `RUXEL_WORKLOAD_DIR` is unset, which it always is
in CI — so "CI is green" does **not** mean "fidelity parity ran". (3) There is
**no advisory or license gate** (`cargo-deny`/`cargo-audit`) despite an
Apache-2.0 project with a GPL clean-room rule and large transitive subtrees. A
reader trusts green CI to mean more than it does, and a vulnerable/copyleft
transitive crate would enter undetected.

## Current state

`.github/workflows/ci.yml` (full file is short; key facts):
- Jobs: `test` (line 18), `fmt` (73), `clippy` (87), `agent-cross` (116).
- `test` job: sets up sccache (`:26-36`, `RUSTC_WRAPPER=sccache`), caches the
  cargo registry (`:43-53`) **and** `target/` (`:55-63`), then runs
  `cargo nextest run --all-features --color=always --no-tests=pass` (`:65`)
  followed by `cargo build --color=always` (`:67`).
- `clippy` job (`:87-114`): caches only the registry (`:102-112`), **no
  `target/` cache, no sccache**; runs `cargo clippy --all-targets -- -D warnings`.
- `agent-cross` job (`:116-147`): caches only the registry (`:131-141`), **no
  `target/` cache, no sccache**; runs `cargo zigbuild --target
  x86_64-unknown-linux-musl --release -p ruxel-agent`.
- Target cache key (`:59`): `cargo-target-${{ runner.os }}-${{ github.head_ref
  || github.ref_name }}-test-${{ hashFiles('**/Cargo.lock') }}` — the
  `head_ref` segment gives every branch its own primary key (cache churn).
- Redundant compile: `cargo nextest run` (`:65`) already builds all targets;
  the trailing `cargo build` (`:67`) recompiles the non-test profile.

Silent-skip gates:
- `crates/ruxel-core/tests/workload.rs:14-17` and `:80-82`:
  ```rust
  let Ok(dir) = std::env::var("RUXEL_WORKLOAD_DIR") else {
      eprintln!("… skipping workload compile gate");
      return;   // <-- counts as PASSED
  };
  ```
- `crates/ruxel-core/tests/render_parity.rs:168-171`: same early-`return` for
  the 41-template parity gate. (The 242-expression gate,
  `expressions_and_conditions_match_oracle`, replays committed goldens and
  **does** run in CI — that one is genuinely covered.)
- `RUXEL_WORKLOAD_DIR` points at a **private** repo checkout
  (`~/Projects/ChainArgos/java-monorepo/ansible-configs`), so CI cannot set it
  without a deploy key. This plan makes the skip **honest** (visible as
  skipped, not passed) and documents the variable — it does **not** wire the
  private checkout into CI (that needs an operator secret; note it for later).

Tooling absence:
- `cargo audit`: not installed locally; no CI step. `cargo-deny`: installed on
  the dev machine but no `deny.toml` and no CI step.
- `--no-tests=pass` (`:65`) greens a binary that discovers zero tests; the
  `ruxel-cli` lib target currently discovers 0 tests, so a whole crate going
  test-empty would be masked.

**Convention**: CI actions are SHA-pinned (keep that); the repo uses
`jdx/mise-action` to provision the toolchain and `mozilla-actions/sccache-action`.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Validate workflow YAML | `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"` | exit 0 (parses) |
| Local deny (if installed) | `cargo deny check advisories bans licenses sources` | exit 0 or a triaged report |
| Confirm skip behavior | `RUXEL_WORKLOAD_DIR= cargo test -p ruxel-core --test workload 2>&1 \| grep -i skip` | shows the skip message today |

Note: you cannot run GitHub Actions locally; verification of the workflow is by
YAML-parse + careful diff against the `test` job's working cache block.

## Scope

**In scope**:
- `.github/workflows/ci.yml` (add caches to `clippy`/`agent-cross`, fix the
  cache key, add a `deny` job, reconsider the redundant build)
- `crates/ruxel-core/tests/workload.rs` and `tests/render_parity.rs` (convert
  silent early-`return` to `#[ignore]` so skips are *visible*)
- `deny.toml` (create at repo root)
- `README.md` dev section (document `RUXEL_WORKLOAD_DIR`)

**Out of scope**:
- Wiring the private `ansible-configs` checkout into CI (needs an operator
  secret; leave a `# TODO(operator)` note in ci.yml instead).
- Changing what clippy/fmt check, or the toolchain pins.
- The `--no-tests=pass` removal is optional (Step 4) — do it only if the
  workspace has no legitimately test-less binary target.

## Git workflow

- Branch: `advisor/004-ci`
- Commit in slices: caches; honest-skip; deny gate. Or one
  `ci: cache clippy/agent-cross, honest fidelity skip, add cargo-deny`.
- Do NOT push/PR unless instructed. (You cannot fully verify CI without a push;
  the operator runs it.)

## Steps

### Step 1: Add `target/` cache + sccache to `clippy` and `agent-cross`

Mirror the `test` job's sccache enable block (`ci.yml:26-36`) and its `target/`
cache step (`:55-63`) into both `clippy` and `agent-cross`, with **distinct
cache keys per job** so they don't collide:
- clippy key suffix: `-clippy-` instead of `-test-`
- agent-cross key suffix: `-musl-` (and include the target triple)

Keep the existing registry-cache steps.

**Verify**: YAML parses (command above); `git diff .github/workflows/ci.yml`
shows the cache blocks added to both jobs with unique keys; the `test` job is
unchanged.

### Step 2: De-churn the target cache key

Change the `test` (and new clippy/agent-cross) target-cache primary key to drop
the `head_ref` segment — key on `${{ runner.os }}` + job + `hashFiles('**/Cargo.lock')`,
letting `restore-keys` provide branch warmth. Concretely, replace
`${{ github.head_ref || github.ref_name }}` with the base branch (`main`) or
omit it, so branches share a warm base cache instead of each seeding a fresh
one.

**Verify**: `grep -n "cargo-target" .github/workflows/ci.yml` → keys no longer
contain `head_ref`.

### Step 3: Make the fidelity-gate skips honest (visible, not "passed")

In `crates/ruxel-core/tests/workload.rs` (both tests) and
`tests/render_parity.rs::template_files_match_oracle`, replace the "read env or
silently `return`" pattern with an **explicit skip that is visible in test
output**. Two acceptable approaches — pick the one that fits the runner:
- Add `#[ignore = "requires RUXEL_WORKLOAD_DIR (private ansible-configs checkout)"]`
  so the test shows as *ignored* rather than passed, and document that CI runs
  it via a separate opt-in step when the checkout is available; **or**
- Keep the runtime check but `eprintln!` a loud `SKIPPED:` line **and** register
  the skip so it's greppable in logs (nextest shows the eprintln).

Prefer `#[ignore]` — it is the honest signal (nextest reports "N skipped").

Add a `# TODO(operator): provide a redacted/real ansible-configs checkout as a
CI secret to run the fidelity gate` comment near the `test` job in ci.yml.

**Verify**: `cargo nextest run 2>&1 | tail -5` shows a non-zero *skipped* count
(not silently absorbed into passed); `RUXEL_WORKLOAD_DIR=<a real dir> cargo
test -p ruxel-core --test workload` still runs the gate when the var is set
(if you have no such dir, confirm by reading the test — do not fabricate one).

### Step 4: Reconsider the redundant `cargo build` and `--no-tests=pass`

- The trailing `cargo build` (`ci.yml:67`) after `cargo nextest run` mostly
  rebuilds already-built targets. Either remove it, or narrow to
  `cargo build --bins` (its only unique value is catching code that compiles
  only under `#[cfg(test)]`, which nextest already covers). If unsure, leave it
  and add a comment explaining why it stays.
- `--no-tests=pass` (`:65`): keep it **only** if some workspace target
  legitimately has no tests. Given plans 016/017 add tests to `ruxel-cli` and
  the agent, prefer dropping it once those land so an accidentally test-empty
  crate fails. For now, leave `--no-tests=pass` but add a
  `# revisit after plans 016/017 add tests to ruxel-cli/agent` comment.

**Verify**: YAML parses; the `test` job still runs nextest.

### Step 5: Add a `cargo-deny` advisory + license gate

Create `deny.toml` at repo root with: advisories (deny vulnerabilities, warn
unmaintained), a license allowlist for the Apache-2.0/MIT/BSD/Unicode/ISC
family this project's deps use, and bans (deny multiple-versions where cheap).
Add a `deny` job to `ci.yml` that installs `cargo-deny` (via a SHA-pinned
action such as `EmbarkStudios/cargo-deny-action`, or via mise) and runs
`cargo deny check advisories bans licenses sources`.

Start permissive on licenses (allow the set the current graph uses — determine
it by reading `Cargo.lock` / `cargo tree`, do not guess) so the first CI run is
green; tighten later. If the advisory DB flags an existing vuln, record it in
the PR description for the operator to triage rather than force-allowing it.

**Verify**: if `cargo-deny` is installed locally, `cargo deny check licenses`
→ exit 0 (allowlist covers the graph); `deny.toml` exists;
`grep -n "cargo deny\|cargo-deny" .github/workflows/ci.yml` → the job is wired.

## Test plan

- No product code changes. The "tests" here are the CI config and the
  visibility of the skip.
- Convert-to-`#[ignore]`: `cargo nextest run` locally must now report the
  workload/template gates as **skipped** (a visible count), and must still
  **pass** everything that ran before (no regressions).
- `deny.toml`: `cargo deny check` (if installed) exits 0 on licenses; advisory
  findings, if any, are triaged in the PR, not silenced.

## Done criteria

ALL must hold:

- [ ] `clippy` and `agent-cross` jobs each have a `target/` cache with a unique key + sccache enabled
- [ ] Target cache keys no longer contain `github.head_ref`
- [ ] `cargo nextest run` reports the workload + template fidelity gates as **skipped** (visible), not passed, when `RUXEL_WORKLOAD_DIR` is unset
- [ ] `deny.toml` exists and a `deny` CI job runs `cargo deny check`
- [ ] `README.md` dev section documents `RUXEL_WORKLOAD_DIR` (what it points at, that it's private, that the gate skips without it)
- [ ] `.github/workflows/ci.yml` parses as valid YAML
- [ ] `plans/README.md` row for 004 updated

## STOP conditions

Stop and report if:
- `cargo deny check advisories` surfaces a HIGH/critical vuln in a runtime
  dependency — report it to the operator; do **not** add a blanket
  `ignore` to make CI pass.
- The license allowlist can't cover the graph without allowing a GPL/copyleft
  crate — that would violate the clean-room rule; report the crate and stop.
- Converting the workload test to `#[ignore]` makes some *other* test fail
  (unexpected coupling) — report it.

## Maintenance notes

- The real fix for the fidelity gate is CI access to the workload (a redacted
  fixture or a deploy-key secret); this plan only makes the gap *visible*. Flag
  it for the operator.
- Reviewer: confirm the license allowlist was derived from the actual `Cargo.lock`
  graph, not copied from a template — an over-broad allowlist defeats the gate.
- After plans 016/017 add tests to `ruxel-cli`/agent, revisit `--no-tests=pass`
  (Step 4) so a test-empty crate fails CI.

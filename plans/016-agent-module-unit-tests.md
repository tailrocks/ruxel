# Plan 016: Unit-test the 18 untested agent modules' pure decision logic

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. This plan
> **adds tests** (and, where needed, extracts pure helpers behind them) — it
> must not change runtime behavior.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/modules/`
> If modules changed, re-verify the coverage matrix; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: L (24 module files; prioritize the riskiest pure logic first)
- **Risk**: LOW (tests; small pure-fn extractions)
- **Depends on**: none (but coordinate with 011/012/014 which add tests to some
  of these files — avoid conflicts by landing after them where they overlap)
- **Category**: tests
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Every agent module's **idempotence decision** — the logic that makes a re-run
safe on the six production servers — is verified today **only** by the manual
on-VM bless-gate over 6 of 16 playbooks. 18 of 24 module files have **zero**
tests; the 6 "tested" ones each cover only a single pure helper. The riskiest
untested logic includes PostgreSQL ACL/flag decisions, `user` supplementary-group
reconciliation, and the destructive storage modules. This plan factors each
module's pure decision logic (SQL/arg builders, diffs, parsers, allowlists)
behind plain functions and table-tests them, so a regression is caught in CI
instead of on a production host. The genuinely root/DB-dependent `apply` stays
for the on-VM gate.

## Current state — coverage matrix (test = inline `#[cfg(test)]`)

| Module (lines) | Tested today | Highest-value untested pure logic |
|---|---|---|
| postgresql.rs (674) | `scram_stored_key`, `b64` only | `validate_privs`, `lit`/`ident`, `grant_sql`, `privs_missing_*`, `wanted_flag`/`flags_changed` parse, `password_changed` verifier parse |
| apt.rs (252) | `summary_changed` only | `missing_packages`, `latest` candidate compare |
| command.rs (151) | `shlex_split` | creates/removes/rc result shape |
| authorized_key.rs (105) | `key_material` | option parse / exclusive / absent |
| sysctl.rs (126) | `normalized` | file-line rewrite, `read_sysctl` path build |
| slurp.rs (57) | `b64` | — |
| **user.rs (198)** | none | supplementary-group append-vs-exact reconcile; passwd/group parse |
| **mount (94)** | none | `opts_eq`, fstab field match/normalize |
| **lvg (122)** | none | PV set subset/superset decision |
| **lvol (102)** | none | size flag/`+`-strip (plan 012), extend decision |
| **filesystem (97)** | none | fstype allowlist (plan 014), grow decision |
| **git (127)** | none | flag-smuggling guards, HEAD compare |
| **get_url (102)** | none | scheme check, dest-exists short-circuit, `--` args |
| **iptables (106)** | none | spec vector build, policy parse |
| **systemd (109)** | none | is-active/is-enabled parse, restarted-always-changed |
| **misc (114)** | none | timezone/group logic |
| **file (85)** | none | state dispatch (plan 012 adds default-state) |
| **copy (52)** | none | force/same/diff decision |
| **lineinfile (77)** | none | verbatim-line-wins rule (the SEMANTICS ⚠) |
| **blockinfile (59)** | none | marker replace vs append |
| **replace (24)** | none | literal-`$` (plan 012) |
| **stat (42)** | none | field mapping |
| **apt_repository (59)** | none | filename validation |
| **mod.rs (276)** | none | `parse_mode` octal, `resolve_uid`/`resolve_gid` passwd parse, `bool_param` coercion, `become_command` runuser wrapping |

**Convention**: inline `#[cfg(test)] mod tests` (see `command.rs:132-151`,
`sysctl.rs:116-126`). No external test deps. For anything touching the real
system (dpkg/systemctl/psql/mkfs/lvm), **do not** test the live path — test the
pure logic (the arg vector, the SQL string, the parse of canned tool output).

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Agent tests | `cargo nextest run -p ruxel-agent` | growing pass count |
| Build | `cargo build -p ruxel-agent` | exit 0 |
| Clippy/fmt | `cargo clippy -p ruxel-agent --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**: `crates/ruxel-agent/src/modules/*.rs` — add `#[cfg(test)] mod
tests`; extract pure helpers (parse/build/compare) from the `run()` bodies where
they're currently inline, keeping behavior identical.

**Out of scope**:
- Any behavior change (pure extraction only — the extracted fn must produce the
  same value the inline code did).
- Testing live dpkg/systemctl/psql/mkfs/lvm — those are on-VM gates.
- The fixes in plans 011/012/014 — this plan tests; if run before them, test
  *current* behavior and let those plans update the assertions.

## Git workflow

- Branch: `advisor/016-module-tests`
- Commit per module or per cluster (e.g. `test(agent): user/mount/lvg/filesystem
  pure-logic tests`).
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Priority tier — DB and destructive storage decision logic

Start with the highest-risk pure logic (a wrong decision here re-grants
privileges or touches partitions):
- **postgresql.rs**: factor and test `validate_privs` (accept/reject),
  `lit`/`ident` quoting (embedded quotes escaped), the SQL string a
  `privs_missing_*`/`grant_sql` builds (assert it references the right catalog
  and filters by role), and the flag-parse helper (plan 011's `wanted_flag`).
- **user.rs**: factor the supplementary-group reconcile (append vs exact) into a
  pure `fn desired_groups(current: &[&str], want: &[&str], append: bool) ->
  Vec<String>` and table-test it (append adds, non-append replaces, dedup); test
  the passwd/group line parse on canned lines.
- **mount.rs**: `opts_eq` (order-insensitive), fstab field-match/rewrite on a
  canned fstab string.
- **lvg.rs**: the PV-set decision (VG exists with subset → extend; superset →
  reduce/error) as a pure set operation on canned `vgs` output.

**Verify**: `cargo nextest run -p ruxel-agent` → new tests pass.

### Step 2: Content and package modules

- **lineinfile.rs**: the **verbatim-line-wins** idempotence rule (SEMANTICS ⚠)
  — pure fn over `(current_content, regexp, line, state)` → `(new_content,
  changed)`; test: line already present unchanged even when regexp matches
  elsewhere; regexp match → replace last; absent → append; state=absent →
  delete.
- **blockinfile.rs**: marker replace vs EOF-append on canned content.
- **copy.rs**: force/same/diff decision (content equal → no change; force:no +
  dest exists → skip).
- **apt.rs**: `missing_packages` and the `latest` candidate-vs-installed compare
  on canned `dpkg-query`/`apt-cache policy` output (feed strings, not fork).
- **apt_repository.rs**: filename validation (reject empty/`/`/`..`).

**Verify**: `cargo nextest run -p ruxel-agent` → pass.

### Step 3: Services, storage helpers, misc, and mod.rs helpers

- **systemd.rs**: parse of `is-active`/`is-enabled` output; `restarted` always
  changed.
- **filesystem.rs**: fstype allowlist (coordinate with plan 014) + grow decision.
- **get_url.rs**: scheme check + `--` arg construction + dest-exists short-circuit.
- **git.rs**: the flag-smuggling guards (reject `-`-prefixed repo/dest/version)
  and the clone-vs-update decision.
- **iptables.rs**: the spec vector built from params (protocol/destination/
  jump/comment) and policy-line parse.
- **misc.rs**: timezone/group logic.
- **stat.rs**: field mapping from `symlink_metadata`.
- **mod.rs**: `parse_mode` (octal string + numeric), `bool_param` coercion
  (yes/true/on/1), `resolve_uid`/`resolve_gid` passwd/group parse on canned
  files (write a scratch `/etc/passwd`-shaped string and point the parser at
  it — you may need to refactor `resolve_uid` to take the passwd content as a
  param for testability; keep a thin wrapper that reads the real file).

**Verify**: `cargo nextest run -p ruxel-agent` → pass.

### Step 4: Full gates + coverage note

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy -p ruxel-agent
--all-targets -- -D warnings` → 0; `cargo nextest run` → green. Record in the PR
which modules now have pure-logic tests and which decision logic remains on-VM-
only (honest coverage statement — don't imply the live paths are unit-tested).

## Test plan

Per-module `#[cfg(test)] mod tests` covering the "highest-value untested" column
of the matrix. Every test is a pure function over constructed inputs (strings,
slices, canned tool output) — **no** forking of real system tools, **no** real
credentials. Model on `command.rs`/`sysctl.rs` existing tests.

## Done criteria

ALL must hold:

- [x] Every module in the matrix with "highest-value untested" logic has at least one pure-logic test (priority-tier modules from Step 1 fully covered)
- [x] Any helper extracted for testability produces byte-identical results to the prior inline code (behavior preserved)
- [x] No test forks a real system tool or uses a real credential
- [x] `cargo nextest run` green; clippy/fmt clean; test count up substantially
- [x] The PR states honestly which logic is unit-tested vs. on-VM-only
- [x] `plans/README.md` row for 016 updated

## Completion evidence (2026-07-16)

Pure helpers now cover SQL/ACL construction, account/group parsing and set
reconciliation, LVM decisions and canned reports, fstab/sysctl/content
rewrites, package status/policy parsing, service state decisions, URL/Git/
iptables argument guards, metadata mapping, and shared parameter parsers. The
agent suite grew from 49 to 74 tests; the full workspace reports 161 passed and
5 fixture-dependent skips. Live dpkg/systemctl/psql/mkfs/LVM execution remains
on the disposable-VM gate only; these unit tests deliberately do not claim to
exercise privileged external tools.

## STOP conditions

Stop and report if:
- Extracting a pure helper would change behavior (e.g. the inline code has a
  side effect entangled with the decision) — leave it, note it, and test what
  you safely can.
- A module's decision genuinely can't be separated from a live-tool call (e.g.
  it fundamentally needs `vgs` output) — feed canned output to the parse half
  and mark the rest on-VM-only.
- Refactoring `resolve_uid`/`resolve_gid` for testability ripples widely —
  keep the thin real-file wrapper and only unit-test the content-parsing inner
  fn.

## Maintenance notes

- This plan is the safety net for plans 011/012/014's fixes and for plan 021's
  snapshot refactor (which changes how dpkg/systemctl are read — the parse tests
  must survive it).
- Reviewer: verify the honest coverage statement — the danger is a false sense
  that "the modules are tested" when only the pure halves are.
- The `lineinfile` verbatim-line-wins test is especially valuable — that rule is
  a SEMANTICS ⚠ and fstab/pam edits depend on it.

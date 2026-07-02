# Plan 005: Characterization tests for the convergence ledger

> **Executor instructions**: Follow step by step; run every verify command.
> Honor STOP conditions. Update this plan's row in `plans/README.md` when done.
> This plan **adds tests only** — do not change `ledger.rs` behavior here (the
> fixes are plans 006/007/008; this net must pass against *today's* code first,
> documenting current behavior, including the known-buggy behavior).
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/ledger.rs crates/ruxel-agent/src/main.rs`
> If either changed, re-verify the excerpts before writing tests; on mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none (prerequisite for 006/007/008)
- **Category**: tests
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

The convergence ledger is the feature ruxel exists for ("plan in seconds") and
the most dangerous subsystem: `cached_ok()` returns a replayed `changed:false`
result and the agent **skips invoking the module entirely**, so any bug that
makes a probe under-capture state means a drifted production host silently
reports `ok` and is never re-converged. Today `ledger.rs` (281 lines) has
**zero tests**. Plans 006/007/008 will change the probe schema and the
record/replay path; without a characterization net, one of those changes could
ship a *new* silent-drift false-positive with green CI. This plan pins the
current record→replay→verify contract so the later fixes are provably safe.
Some tests here will document **current buggy behavior** (e.g. attrs ignored) —
that is intentional; plan 006 flips those assertions when it fixes the bug.

## Current state

- `crates/ruxel-agent/src/ledger.rs` — the ledger. Key surface (all private
  except the `Ledger` type; you will test through the public API + add a small
  test module inside the file):
  - `pub struct Ledger { path, records: HashMap<String,Record>, dirty }`
  - `Ledger::load(state_dir: &Path) -> Self` (`:77-88`) — reads
    `<state_dir>/ledger/ledger.json`; on missing/corrupt JSON → empty via
    `unwrap_or_default()` (`:79-83`).
  - `Ledger::flush(&self)` (`:90-103`) — no-op if `!dirty`; else atomic
    tmp+rename.
  - `Ledger::cached_ok(&self, key) -> Option<Value>` (`:108-121`) — returns the
    replayed result (with `changed` forced false) iff a record exists, its
    `agent_version` matches `env!("CARGO_PKG_VERSION")`, and **every** probe
    `verify()`s; returns `None` if probes empty or any fails.
  - `Ledger::record(&mut self, key, module, params, status, result)`
    (`:125-152`) — no-ops for empty key / status `failed`|`skipped` / modules
    whose `probe_for` returns `None` / empty probe set; else inserts a `Record`
    and sets `dirty`.
  - `fn probe_for(module, params) -> Option<Vec<Probe>>` (`:157-211`) — the
    cacheable-module table. `enum Probe { File{path,sha256,len}, Pkg{name,version},
    Unit{name,active,enabled}, SysctlKV{file,name,value} }` with `verify()`
    (`:44-59`).
  - Helper fns hitting the real system: `file_fingerprint` (reads the file),
    `dpkg_version` (forks `dpkg-query`), `unit_active`/`unit_enabled` (fork
    `systemctl`), `sysctl_file_value` (reads the file).
- **Testability note**: `probe_for` for `file`/`copy`/`template` computes a
  `File` probe by hashing the path; `SysctlKV` reads a file. Both are testable
  with a `tempfile`-style scratch dir. `Pkg`/`Unit` probes fork system tools —
  do **not** test those against the real system (they'd be environment-flaky
  and could touch package/unit state). Test the file-backed probes end-to-end
  and the record/replay bookkeeping for all; treat dpkg/systemctl probes as
  out of scope for unit tests (they belong to the on-VM bless-gate).

- **Test conventions in this repo**: inline `#[cfg(test)] mod tests` with
  `#[test]` fns (see `crates/ruxel-agent/src/modules/command.rs:132-151` and
  `sysctl.rs:116-126` for the pattern). No external test framework beyond std +
  `serde_json`. For a scratch directory, use `std::env::temp_dir().join(...)`
  with a unique subdir per test (the repo avoids adding dev-dependencies to the
  agent to keep the static-musl footprint small — do **not** add `tempfile`;
  make a unique dir under `temp_dir()` and clean it up, mirroring how the
  protocol test manages `RUXEL_STATE_DIR`).

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Run ledger tests | `cargo nextest run -p ruxel-agent ledger` | new tests pass |
| Or with std harness | `cargo test -p ruxel-agent ledger` | new tests pass |
| Clippy | `cargo clippy -p ruxel-agent --all-targets -- -D warnings` | exit 0 |
| Fmt | `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel-agent/src/ledger.rs` — add a `#[cfg(test)] mod tests` at the
  bottom. You MAY add narrowly-scoped `#[cfg(test)]` test helpers or make one
  or two private items `pub(crate)`/`pub(super)` **only if strictly needed** to
  test them (prefer testing through `load`/`record`/`cached_ok`/`flush`).

**Out of scope**:
- Any behavior change to `ledger.rs` (that's 006/007/008). If you notice a bug,
  write a test that pins the **current** (buggy) behavior and add a
  `// BUG(plan 006): current behavior; will flip when attrs are probed` comment.
- Adding dev-dependencies (`tempfile` etc.) — use `std::env::temp_dir()`.
- Testing the `dpkg-query`/`systemctl`-backed probes against a real system.

## Git workflow

- Branch: `advisor/005-ledger-tests`
- One commit: `test(ledger): characterization tests for record/replay/verify`
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Add a scratch-dir test harness helper

At the bottom of `ledger.rs`, add `#[cfg(test)] mod tests` with a helper that
creates a unique temp dir (e.g. `temp_dir().join(format!("ruxel-ledger-test-{}-{}",
process::id(), <a per-test counter or the test name>))`), returns its path, and
a matching cleanup (`remove_dir_all`, ignore errors). Since
`Date.now()`/random aren't needed here, key uniqueness on the test-fn name
string passed in.

**Verify**: `cargo test -p ruxel-agent ledger 2>&1 | tail -3` compiles and runs
(even with just the helper + one trivial test).

### Step 2: Pin the load/flush round-trip

Tests:
- `load_missing_ledger_is_empty`: `Ledger::load` on a fresh dir yields a ledger
  whose `cached_ok(any)` is `None`.
- `flush_noop_when_not_dirty`: `load` then `flush` writes **no** file (assert
  `ledger/ledger.json` does not exist).
- `record_then_flush_then_load_roundtrips`: record a `file` task against a real
  scratch file, `flush`, then `load` a fresh `Ledger` from the same dir and
  assert `cached_ok(key)` returns `Some` with `changed == false`.
- `corrupt_ledger_json_loads_empty`: write garbage bytes to
  `<dir>/ledger/ledger.json`, `load`, assert it does **not** panic and
  `cached_ok(any)` is `None` (pins the `unwrap_or_default` recovery at `:79-83`).

**Verify**: `cargo nextest run -p ruxel-agent ledger` → these pass.

### Step 3: Pin the `cached_ok` verdict logic (File probe)

Using a scratch file with known content:
- `cached_ok_hits_when_content_unchanged`: record a `copy`/`file` task pointing
  at the scratch file; `cached_ok(key)` → `Some`, and the returned JSON has
  `changed == false`.
- `cached_ok_misses_when_content_changed`: after recording, mutate the file's
  **content**; `cached_ok(key)` → `None` (fingerprint mismatch → re-check).
- `cached_ok_misses_on_agent_version_change`: construct a record whose
  `agent_version` differs from `env!("CARGO_PKG_VERSION")` (e.g. flush a record,
  hand-edit the JSON's `agent_version` field to `"0.0.0-test"`, reload) and
  assert `cached_ok` → `None` (pins `:110`).
- **BUG-pinning test** `cached_ok_hits_even_when_mode_changed`: record a `file`
  task; then `chmod` the scratch file (change only permissions, not content);
  assert `cached_ok(key)` still returns `Some` (i.e. attrs are NOT probed
  today). Add comment: `// BUG(plan 006, CORRECTNESS-01): File probe ignores
  mode/owner; plan 006 flips this to None.` (On non-Unix this test can't run —
  gate it `#[cfg(unix)]`.)

**Verify**: `cargo nextest run -p ruxel-agent ledger` → all pass, including the
BUG-pinning one (which documents today's behavior).

### Step 4: Pin the `record` honesty-rule gating

- `record_skips_failed_and_skipped`: `record(key, "file", params, "failed", ...)`
  and `record(key, "file", params, "skipped", ...)` both leave the ledger with
  `cached_ok(key) == None` (nothing recorded).
- `record_skips_noncacheable_modules`: `record(key, "command", ...)`,
  `record(key, "shell", ...)`, and a `systemd` task with `state=restarted`
  leave `cached_ok(key) == None` (probe_for returns `None` → honesty rule).
- `record_skips_empty_key`: `record("", "file", ...)` records nothing.
- **BUG-pinning test** `record_caches_apt_latest`: `record(key, "apt", {"name":
  "x", "state": "latest"}, "ok", ...)` — assert that `probe_for` currently
  **does** produce probes for this (i.e. `cached_ok` can later hit). To pin
  without a real dpkg, assert via `probe_for` directly if you expose it
  `pub(super)`, OR document via a comment that this path forks `dpkg-query` and
  is covered by the on-VM gate. Add: `// BUG(plan 006, CORRECTNESS-02): apt
  state=latest should be network-truth (never cached); plan 006 returns None.`

**Verify**: `cargo nextest run -p ruxel-agent ledger` → all pass.

### Step 5: Confirm full gates stay green

**Verify**:
- `cargo fmt --all --check` → exit 0
- `cargo clippy -p ruxel-agent --all-targets -- -D warnings` → exit 0
- `cargo nextest run` → whole suite passes, with N new ledger tests visible.

## Test plan

All tests are new, in `crates/ruxel-agent/src/ledger.rs`'s `#[cfg(test)] mod
tests`, modeled structurally on the existing inline tests in
`crates/ruxel-agent/src/modules/command.rs` and `sysctl.rs`. Coverage:
load/missing/corrupt, flush no-op + roundtrip, cached_ok hit/miss/version-miss,
the attrs-ignored BUG pin, record honesty-rule gating, and the apt=latest BUG
pin. Two tests deliberately assert **current buggy** behavior and are labeled
`BUG(plan 006 ...)` so plan 006 knows exactly which assertions to flip.

## Done criteria

ALL must hold:

- [ ] `cargo nextest run -p ruxel-agent ledger` runs ≥ 10 new tests, all pass
- [ ] `cargo fmt --all --check` exits 0
- [ ] `cargo clippy -p ruxel-agent --all-targets -- -D warnings` exits 0
- [ ] The two `BUG(plan 006 ...)` tests exist and pass (documenting current behavior)
- [ ] No behavior change to `ledger.rs` (only a `#[cfg(test)]` module and, if
      unavoidable, `pub(super)` on ≤2 items) — `git diff` shows no logic edits
- [ ] `plans/README.md` row for 005 updated

## STOP conditions

Stop and report if:
- Testing the file-backed probes requires making more than ~2 private items
  visible, or requires an external crate — reconsider the approach and report.
- A "current behavior" test you expected to pass actually fails — the code may
  already differ from the excerpts (drift); re-run the drift check.
- Any test would fork `dpkg-query`/`systemctl` against the real machine — do
  not; those are out of scope (STOP and note it).

## Maintenance notes

- Plan 006 will **flip** the two `BUG(...)`-labeled assertions when it fixes
  CORRECTNESS-01 (attrs) and CORRECTNESS-02 (apt=latest). The comments tell the
  006 executor exactly where.
- Keep these tests hermetic (temp dirs, `#[cfg(unix)]` where needed) so they run
  in CI without a VM.
- Reviewer: confirm no test mutates real system package/unit state.

# Plan 006: Fix ledger silent-drift probes (attrs, apt=latest, sysctl live)

> **Executor instructions**: Follow step by step; run every verify command.
> Honor STOP conditions. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/ledger.rs`
> If it changed since `b5f98ba`, re-verify the excerpts; on mismatch, STOP.
> Also confirm plan 005's tests are merged (`git log --oneline | grep -i "005\|characterization"`) — this plan flips two of them.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW (tightens verification — worst case a few extra module re-runs, never a wrong "changed")
- **Depends on**: 005 (its characterization tests are the safety net; two get flipped here)
- **Category**: bug (correctness — silent production drift)
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

The ledger replays a cached `changed:false` result and **skips invoking the
module** when its probes still verify. Three probes under-capture state, so a
drifted server reports `ok (ledger)` and is never re-converged:
1. **CORRECTNESS-01**: the `File` probe records only content hash + length, not
   `mode`/`owner`/`group`. A converged `copy`/`template`/`file` task with
   `mode: "0600"` that someone `chmod 0644`s re-verifies the content hash → cache
   hit → the declared mode is never restored. Silent permission/ownership drift
   on config and secret files. (Also: `file state=directory` currently disables
   caching entirely because `file_fingerprint` does `std::fs::read` on a dir and
   fails — so directory attr tasks re-run every time; fixable in the same edit.)
2. **CORRECTNESS-02**: the `apt` probe fingerprints the installed version with
   **no check of `state`**, so `state: latest` gets cached. Once converged, a
   newer upstream package is never detected — `latest` silently degrades to
   `present`, violating ARCHITECTURE §6's network-truth honesty rule.
3. **CORRECTNESS-10**: the `sysctl` probe checks only the file value, not the
   live `/proc/sys` value that the module enforces when `sysctl_set: true`. If
   the file matches but the live value drifted, cache hit → live value never
   re-applied.

These are the worst bug class for this tool. The fix strictly tightens
verification.

## Current state

`crates/ruxel-agent/src/ledger.rs`:

- The probe enum + verify (`:18-59`):
  ```rust
  enum Probe {
      File { path: String, sha256: String, len: u64 },
      Pkg { name: String, version: String },
      Unit { name: String, active: bool, enabled: bool },
      SysctlKV { file: String, name: String, value: String },
  }
  // verify(): File → file_fingerprint(path) matches sha256 && len;
  //           SysctlKV → sysctl_file_value(file,name) == value; etc.
  ```
- `probe_for` (`:157-211`):
  - File modules arm (`:160-172`): for `file|copy|template|lineinfile|replace|
    blockinfile`, returns `None` for `state: absent|link`, else
    `file_fingerprint(path)?` → one `File` probe (no attrs).
  - apt arm (`:173-186`): builds one `Pkg{name,version}` per installed package
    via `dpkg_version`; returns `None` only when the name list is empty. **No
    `state` inspection.**
  - sysctl arm (`:199-208`): one `SysctlKV{file,name,value}` from
    `sysctl_file_value`. **No live-value probe.**
- `apply_attrs` in `crates/ruxel-agent/src/modules/mod.rs:242-276` is what the
  module uses to enforce mode/uid/gid — mirror its comparison logic in the new
  probe (mode via `PermissionsExt::mode() & 0o7777`, uid/gid via `MetadataExt`).
- `sysctl_file_value` (`:266-280`) and `read_sysctl` equivalent: note the module
  reads the live value at `crates/ruxel-agent/src/modules/sysctl.rs:109-114`
  (`/proc/sys/<name with . → />`). Reuse that path shape.

**Convention**: probes are `#[serde(tag = "kind")]` enum variants that
serialize into `ledger.json`. Adding fields to `File`/`SysctlKV` or a new
variant is a **ledger format change** — old records won't have the new fields.
Because `serde_json` deserialization of a struct variant fails when required
fields are missing, and `load` swallows a corrupt/incompatible ledger into an
empty map (`:79-83`), old ledgers degrade gracefully to "re-check everything"
(safe). Still, bump the honesty signal (see Step 4).

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Ledger tests | `cargo nextest run -p ruxel-agent ledger` | pass (incl. flipped assertions) |
| Clippy | `cargo clippy -p ruxel-agent --all-targets -- -D warnings` | exit 0 |
| Fmt | `cargo fmt --all --check` | exit 0 |
| Cross-build agent (musl) | `cargo build -p ruxel-agent` (or zigbuild per RESTORE) | exit 0 |

## Scope

**In scope**:
- `crates/ruxel-agent/src/ledger.rs` — the `Probe` enum, `verify`, `probe_for`,
  and helpers.
- `crates/ruxel-agent/src/ledger.rs`'s `#[cfg(test)] mod tests` — flip the two
  `BUG(plan 006 ...)` assertions from plan 005 and add new positive tests.

**Out of scope**:
- The `Pkg`/`Unit` probe internals (dpkg/systemctl) — only the apt `state`
  guard changes, not how versions are read.
- `no_log`/at-rest concerns — plan 008.
- Flush-on-interrupt — plan 007.
- The module `apply_attrs` logic — reuse it as the reference; don't change it.

## Git workflow

- Branch: `advisor/006-ledger-probes`
- Commit per fix or one `fix(ledger): probe file attrs, honor apt=latest, add sysctl live probe`.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Extend the `File` probe with mode/uid/gid and support directories

Add `mode: u32`, `uid: u32`, `gid: u32` fields to `Probe::File` (keep
`sha256`/`len` for regular files; for directories there is no content hash —
see below). Update `verify()` for `File` to also compare
`symlink_metadata(path)`'s `mode() & 0o7777`, `uid()`, `gid()` against the
recorded values, in addition to content. In `probe_for`'s file-modules arm:
- Compute mode/uid/gid from `symlink_metadata` for every path.
- For `state: directory` (currently falls through to `file_fingerprint` which
  fails on a dir → `None`): record a directory-appropriate probe. Simplest:
  make the content fields optional (`sha256: Option<String>`, `len: Option<u64>`)
  and set them `None` for directories, comparing only attrs; OR add a distinct
  `Dir { path, mode, uid, gid }` variant. Prefer optional content fields to keep
  one variant. Verify accordingly (skip content compare when `sha256` is `None`).
- Keep returning `None` for `state: absent|link` (unchanged).

**Verify**: the plan-005 test `cached_ok_hits_even_when_mode_changed` now
**fails** with the old assertion — change it to
`cached_ok_misses_when_mode_changed` (assert `None` after a `chmod`). Add
`cached_ok_hits_when_dir_attrs_unchanged` and
`cached_ok_misses_when_dir_owner_changed` (using `#[cfg(unix)]`, a scratch dir,
`std::os::unix::fs::chown` may need root — if so, test the mode change on a dir
instead of owner). `cargo nextest run -p ruxel-agent ledger` → pass.

### Step 2: Stop caching `apt state=latest`

In `probe_for`'s apt arm (`:173-186`), read `params.get("state")`; if it equals
`"latest"`, return `None` (network-truth class — the module's `apt-cache
policy` check must run every time). Also return `None` when `update_cache` or
`upgrade` is present with no installable `name` (these already fall out via the
empty-names guard, but assert it). Leave `state: present` caching intact.

**Verify**: the plan-005 `record_caches_apt_latest` BUG test → flip to
`apt_latest_is_not_cached` asserting `probe_for("apt", {"state":"latest",...})`
returns `None`. To test `probe_for` directly you may need it `pub(super)` —
acceptable. `cargo nextest run -p ruxel-agent ledger` → pass.

### Step 3: Add a live-value probe for `sysctl_set`

Add a `SysctlLive { name: String, value: String }` variant (or a
`live: Option<String>` field on `SysctlKV`). In `probe_for`'s sysctl arm, when
`params["sysctl_set"]` is truthy (mirror `bool_param` semantics from
`modules/mod.rs:181-190`), additionally read the live value via
`/proc/sys/<name with '.'→'/'>` (reuse the path shape from
`modules/sysctl.rs:109-114`) and record it. `verify()` compares the live value
using the **same whitespace-normalization** the module uses
(`modules/sysctl.rs:105-107` `normalized`) — copy that helper or factor it so
both agree; a naive `==` would cause false cache misses on multi-value keys.

**Verify**: add `sysctl_set_live_drift_misses_cache` (write a scratch sysctl
file that matches, but there's no safe way to mutate `/proc/sys` in a unit test
— so test the `verify()` logic on a constructed `SysctlLive` probe with a
mismatching value → `false`, matching normalized value → `true`). `cargo
nextest run -p ruxel-agent ledger` → pass.

### Step 4: Bump the ledger format signal

Because the probe schema changed, records written by an older agent lack the
new fields. Confirm graceful degradation: an old `ledger.json` should
deserialize to empty (safe re-check), not panic. If `serde` would error on the
whole file, `load`'s `unwrap_or_default` already catches it (`:79-83`) — verify
by test (`corrupt_ledger_json_loads_empty` from plan 005 already covers the
"unparseable → empty" path; add `old_schema_ledger_loads_empty_or_ignored`
feeding a JSON record missing the new `mode` field and asserting no panic and
`cached_ok → None`). This is the honest-degradation guarantee.

**Verify**: `cargo nextest run -p ruxel-agent ledger` → the old-schema test
passes (no panic; treated as miss).

### Step 5: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy -p ruxel-agent
--all-targets -- -D warnings` → 0; `cargo nextest run` → whole suite green.

## Test plan

Flip the two `BUG(plan 006 ...)` tests from plan 005 to assert the corrected
behavior, and add:
- `cached_ok_misses_when_mode_changed` (File attr drift → re-check)
- directory attr hit/miss tests (`#[cfg(unix)]`)
- `apt_latest_is_not_cached` (probe_for returns None)
- `SysctlLive::verify` mismatch/normalized-match unit tests
- `old_schema_ledger_loads_empty_or_ignored` (graceful degradation)
Model structurally on plan 005's harness (scratch temp dir, no external crates).

## Done criteria

ALL must hold:

- [x] `Probe::File` includes mode/uid/gid; `verify()` compares them; directories are probeable (not force-uncached)
- [x] `probe_for` returns `None` for `apt` with `state: latest`
- [x] `sysctl_set` tasks record and verify the live `/proc/sys` value with whitespace normalization
- [x] Old-schema `ledger.json` degrades to "re-check" without panic
- [x] The two plan-005 `BUG(...)` tests are flipped to assert correct behavior and pass
- [x] `cargo nextest run` green; `cargo clippy --all-targets -- -D warnings` exits 0; `cargo fmt --all --check` exits 0
- [x] `plans/README.md` row for 006 updated

## STOP conditions

Stop and report if:
- Making `File` content optional forces a large ripple through `verify`/record
  that risks the `Pkg`/`Unit` paths — reconsider (a separate `Dir` variant may
  be cleaner) and report your chosen shape.
- The whitespace-normalization helper can't be shared between agent module and
  ledger without a larger refactor — inline a copy with a comment and note the
  duplication for plan 018 (DEBT-08 constants), don't over-engineer here.
- A directory-owner test requires root and there's no non-root equivalent —
  drop that specific assertion, keep the mode-change one, and note it.

## Maintenance notes

- Any new cacheable module added later must add attrs to its probe if it manages
  file permissions — the File-probe attr comparison is the template.
- Reviewer: the load-bearing property is "a fingerprint match never lets a
  drifted attribute survive." Scrutinize `verify()` for every field the module's
  `apply` can change.
- Plan 021 (system snapshots) will change how `dpkg_version`/`unit_*` are read;
  it must preserve these probe semantics (the apt=latest guard especially).

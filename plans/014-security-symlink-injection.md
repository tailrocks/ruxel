# Plan 014: Close symlink-follow + injection surfaces in write/exec modules

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. This is a
> **security** plan — be precise; never write a secret into a file/test/commit.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/modules/`
> If the modules changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P1 (authorized_key) / P2 (rest)
- **Effort**: M
- **Risk**: MED (changes how privileged writes happen; must preserve
  ownership/create behavior — guarded by tests + on-VM gate)
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Several agent modules perform privileged writes/execs that **follow symlinks**
or build a program name from a rendered param — surfaces Ansible closes with
atomic in-directory rename + allowlists. The highest-severity is
`authorized_key`: it `write`s, `chmod`s, and `chown`s `~user/.ssh/authorized_keys`
following symlinks, so a pre-planted symlink (in a compromised service account's
home) makes root truncate and `chown` an attacker-chosen file to the unprivileged
uid — a classic symlink→chown privilege escalation. The content modules
(`copy`/`template`/`lineinfile`/`replace`/`blockinfile`/`sysctl`/`mount`) write
via `std::fs::write` on rendered paths (also symlink-following), and some build
config lines where an embedded newline injects extra entries. `filesystem`
builds `mkfs.{fstype}` from a rendered param with no allowlist. These are
defensive-maintenance fixes: identify the pattern, make the write safe, add an
allowlist. Note: tasks legitimately run arbitrary root actions **by design**
(shell/command); the fixes here target *unintended* escalation surfaces in
*other* modules, at parity with Ansible.

## Current state

**A. `authorized_key` symlink-follow (SECURITY-05, highest):**
`crates/ruxel-agent/src/modules/authorized_key.rs:33-51`:
```rust
if !ssh_dir.exists() {
    std::fs::create_dir_all(&ssh_dir)...;
    set_permissions(&ssh_dir, 0o700)...;
    chown(&ssh_dir, uid, gid)...;
}
let mut next = current.clone(); ...
std::fs::write(&auth_file, next)...;               // follows a symlink at auth_file
std::fs::set_permissions(&auth_file, 0o600)...;    // follows
std::os::unix::fs::chown(&auth_file, uid, gid)...; // chown (not lchown) → follows
```
The `!ssh_dir.exists()` guard is skipped entirely when `.ssh` already exists, and
the write/chown follow any symlink at `authorized_keys`. Ansible writes a temp
file in-dir and atomically renames (replacing the link, not following it).

**B. Content modules `std::fs::write` (symlink-follow) + newline injection
(SECURITY-11):**
- `copy.rs:43-44` (temp+rename already — copy is actually the *good* pattern:
  `.ruxel-tmp` then `rename`; confirm and use it as the model), but the tmp name
  is predictable and it does not refuse a symlinked final path.
- `lineinfile.rs`, `blockinfile.rs:51`, `replace.rs:21`, `sysctl.rs:62`,
  `mount.rs:54` — direct `std::fs::write(path, ...)` (follows symlinks).
- Newline injection: `sysctl.rs:49,57` build a `name=value` line and
  `mount.rs:26` builds an fstab line from params; an embedded newline in a value
  injects an additional config entry.

**C. `filesystem` mkfs from rendered param (SECURITY-07):**
`filesystem.rs:52-53`:
```rust
fn make(dev: &str, fstype: &str) -> Result<(), String> {
    let bin = format!("mkfs.{fstype}");   // fstype is a rendered param → program name
```
No allowlist; the workload uses only `xfs`/`ext4`.

**Convention**: `copy.rs` already demonstrates the atomic in-dir temp+rename
pattern (`copy.rs:37-44`). Factor a shared safe-write helper (this also serves
plan 018's DEBT-02 consolidation). `apply_attrs` (`modules/mod.rs:242-276`) is
the attr helper; note it uses `symlink_metadata` (good) but `chown` (follows).

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Agent tests | `cargo nextest run -p ruxel-agent` | pass |
| Build | `cargo build -p ruxel-agent` | exit 0 |
| Clippy/fmt | `cargo clippy -p ruxel-agent --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**: `crates/ruxel-agent/src/modules/{authorized_key,copy,lineinfile,
blockinfile,replace,sysctl,mount,filesystem}.rs`, and a shared safe-write helper
in `crates/ruxel-agent/src/modules/mod.rs`.

**Out of scope**:
- `shell`/`command` executing arbitrary root actions — that's the product, not a
  bug.
- The `apply_attrs` `chown`→`lchown` question for non-authorized_key modules —
  address it only where a symlink-follow is a real escalation (authorized_key);
  for content files the safe-write (temp+rename) removes the follow.
- Adding modules/params.

## Git workflow

- Branch: `advisor/014-security-writes`
- Commit per lettered fix; A (authorized_key) first and standalone (highest).
- Do NOT push/PR unless instructed.

## Steps

### Step 1 (A, highest): Make `authorized_key` writes symlink-safe

Rewrite the write in `authorized_key.rs:41-51` to:
- Refuse to operate if `~user/.ssh` or `authorized_keys` is a **symlink** or the
  `.ssh` dir is not owned by the target user (check via `symlink_metadata` +
  `MetadataExt::uid`). Return an error rather than following.
- Write to a temp file **in the same directory** with `O_EXCL`
  (`OpenOptions::new().create_new(true).write(true)`), set its mode to 0600 and
  `fchown`/`chown` the fd/temp (the temp is freshly created, not a symlink),
  then `rename` it over `authorized_keys` (atomic; replaces a link instead of
  following it).
- Preserve the existing create-`.ssh`-dir behavior (0700, owned by user) when
  absent.

**Verify**: add `#[cfg(unix)]` tests (using a scratch dir):
- `authorized_key_refuses_symlinked_target`: pre-create `authorized_keys` as a
  symlink to another file; the module returns an error and the pointed-to file
  is **unchanged** (not truncated/chowned).
- `authorized_key_writes_atomically`: normal case appends the key, file is 0600.
Some assertions (chown to another uid) need root — gate those `#[cfg(unix)]` and
skip the chown assertion when not root, keeping the symlink-refusal assertion
(which needs no root). `cargo nextest run -p ruxel-agent authorized_key` → pass.

### Step 2 (B): Add a shared safe-write helper and route content modules through it

Add `pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String>`
in `modules/mod.rs` that writes to an in-dir temp (`.<name>.ruxel-tmp` or a
random suffix), then `rename`s over `path`. Before writing, if `path` exists and
is a **symlink**, decide per Ansible parity: content modules in Ansible follow
symlinks to regular files by default *but* write atomically (rename replaces the
link). The safest parity-preserving behavior is temp+rename (which does **not**
follow — it replaces the link). Route `lineinfile`, `blockinfile.rs:51`,
`replace.rs:21`, `sysctl.rs:62`, `mount.rs:54`, and `copy.rs` (reuse its
existing pattern or switch to the shared helper) through `write_atomic`.

For **newline injection**: in `sysctl.rs` (name/value) and `mount.rs` (the 6
fstab fields), reject a value containing `\n` or `\r` before writing (a
single-line config field must not carry a newline). Return a clear error.

**Verify**: `content_write_replaces_symlink_atomically` test; `sysctl_rejects_newline_value`
and `mount_rejects_newline_field` unit tests. `cargo nextest run -p ruxel-agent`
→ pass.

### Step 3 (C): Allowlist `filesystem` fstype

In `filesystem.rs::make` (`:52-53`), validate `fstype` against the closed set
`{"xfs", "ext4"}` before building `mkfs.{fstype}`; reject anything else with an
"outside the closed surface" error. (The `run()` fn already compares fstype, but
`make` is where the program name is built — guard there.)

**Verify**: unit test `filesystem_rejects_unknown_fstype` (a `fstype` of
`"; rm -rf"` or `"btrfs"` → Err before any exec). `cargo nextest run -p
ruxel-agent filesystem` → pass.

### Step 4: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy -p ruxel-agent
--all-targets -- -D warnings` → 0; `cargo nextest run` → green.

## Test plan

- authorized_key: symlink-refusal (no root needed) + atomic write.
- content modules: symlink-replacement (not follow) via `write_atomic`; newline
  rejection for sysctl/mount single-line fields.
- filesystem: fstype allowlist reject.
All hermetic (`#[cfg(unix)]`, scratch dirs, no real credentials, no real
mkfs/LVM). Live behavior is the on-VM gate.

## Done criteria

ALL must hold:

- [ ] `authorized_key` refuses a symlinked `.ssh`/`authorized_keys` and writes via in-dir temp + atomic rename (test proves the pointed-to file is untouched)
- [ ] Content modules write via a shared `write_atomic` (temp+rename), not `std::fs::write` on the target path
- [ ] `sysctl`/`mount` reject newline-bearing single-line field values
- [ ] `filesystem` allowlists `fstype ∈ {xfs, ext4}` before building `mkfs.<fstype>`
- [ ] `cargo nextest run` green; clippy/fmt clean
- [ ] `plans/README.md` row for 014 updated

## STOP conditions

Stop and report if:
- Ansible's `authorized_key`/content-module behavior on an existing symlink
  turns out to *require* following (some configs symlink `authorized_keys`
  intentionally) — the safest parity is still temp+rename replacing the link;
  but if a bless-gate shows a divergence, report it before shipping.
- The atomic-rename helper breaks `copy`'s existing diff/attr flow — reconcile
  with `copy.rs`'s current temp+rename (it may already be correct; prefer
  reusing it).
- A test needs root to prove the chown behavior — keep the no-root symlink-refusal
  assertion and note the chown assertion is on-VM only.

## Maintenance notes

- The `write_atomic` helper is reused by plan 018 (DEBT-02 command/IO
  consolidation). Keep it in `modules/mod.rs`.
- Reviewer: the load-bearing property for A is "root never follows an
  attacker-controlled symlink into a privileged write/chown." Verify the temp is
  created with `O_EXCL` in the **same directory** (a `/tmp` temp + rename across
  filesystems would fail and isn't atomic).
- The committed-capture credential (SECURITY-09) and the mux-socket/Debug
  hardening (SECURITY-06/08/10) are in plan 015, not here.

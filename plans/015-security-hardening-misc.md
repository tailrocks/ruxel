# Plan 015: Misc security hardening (mux socket, agent re-hash, Debug, capture)

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. **Security
> plan** — never reproduce a secret value in any file, test, commit, or the PR;
> reference credential *type* and *location* only.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel/src/transport.rs crates/ruxel-core/src/engine.rs tools/oracle/captures/`
> If any changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Four lower-severity hardening items, each defensible on its own and cheap:
the SSH ControlMaster mux socket lives in world-writable `/tmp` with a
predictable name (Ansible uses a user-private 0700 dir); the content-addressed
agent is trusted by **filename** with no re-hash before exec; secret-bearing
scope/memo types derive `Debug` (a latent leak if ever `dbg!`'d); and a
committed oracle capture contains a **credential-shaped literal** (a 9-character
password for a PG role that is neither a `dry-secret` fake nor the known
`s3cr3t-ruxel` test value) plus real `op://` item references and a GCP key
filename. The first three are code hardening; the last is an operator
verify/rotate action this plan surfaces without reproducing the value.

## Current state

**A. Mux socket in `/tmp` (SECURITY-06):**
`crates/ruxel/src/transport.rs:54-61`:
```rust
let socket = std::env::temp_dir().join(format!(
    "ruxel-mux-{}-{:x}",
    std::process::id(),
    ...subsec_nanos()
));
```
`temp_dir()` is `/tmp` absent `$TMPDIR`. Ansible uses `~/.ansible/cp/` (0700).
On a multi-user controller the predictable path in a shared dir invites
pre-creation/DoS races against the socket carrying root SSH sessions.
(Lower severity under the stated single-operator model; still a regression vs
Ansible.) Note: the same `subsec_nanos`-only name is also the multi-host
collision risk flagged for plan 022 — coordinate.

**B. Agent trusted by filename (SECURITY-08):**
`crates/ruxel/src/transport.rs` `ensure_agent` returns early when
`test -x <remote_path>` succeeds (path name = `blake3` of local bytes) and
**never re-hashes** the remote file before spawning it. Integrity rests on the
filename only. Only root can write `/var/lib/ruxel/agent/`, and root is trusted
by design, so this is defense-in-depth, not a live escalation — but the
content-addressing *implies* a check that isn't performed.

**C. `Debug` on secret-bearing types (SECURITY-10):**
`crates/ruxel-core/src/engine.rs` derives `Debug` on `VarValue` (`:154`),
`Scope` (`:162`), and `ScopeObject` (`:204`) — the latter holds
`memo: Mutex<HashMap<String, Value>>` with resolved secrets. No active `{:?}`/
`dbg!` site exists today (grep), so no leak now; it's a latent footgun.
`MemoizedResolver` correctly does **not** derive `Debug`.

**D. Committed capture credential (SECURITY-09):**
`tools/oracle/captures/pg-bless.jsonl` contains a `password` field whose value
is a 9-character literal that is **not** a `dry-secret-*` fake and **not**
`s3cr3t-ruxel` (the known synthetic test value used in `postgresql.rs` tests).
It corresponds to a PG role (the `looker` role in the capture). Other captures
contain real `op://ChainArgos/...` item references and a GCP service-account key
filename — reconnaissance metadata, not secret values. **Do not open the file
to read the value; do not reproduce it anywhere.** Whether the 9-char value is a
real credential depends on the upstream (private) playbook, not this repo.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Build | `cargo build --workspace` | exit 0 |
| Transport tests | `cargo nextest run -p ruxel-cli transport` | pass (ignored gate needs a VM) |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |
| Count password-shaped values (no value printed) | `grep -c '"password"' tools/oracle/captures/pg-bless.jsonl` | a count, not the value |

## Scope

**In scope**:
- `crates/ruxel/src/transport.rs` — socket location (A), agent re-hash (B).
- `crates/ruxel-core/src/engine.rs` — redacting `Debug` impls (C).
- The PR description + `plans/README.md` — surface D for the operator (no code).

**Out of scope**:
- Reading or reproducing the capture's credential value (D is a *report*, not an
  edit — unless the operator confirms it's real and asks for a scrub).
- Rewriting git history to scrub the capture (operator decision; needs
  force-push authorization not granted to autonomous work).
- The multi-host socket-collision fix (plan 022) — only make the socket
  path *private* here; uniqueness-across-hosts is 022's concern.

## Git workflow

- Branch: `advisor/015-security-hardening`
- Commit per item; A/B/C are code, D is PR-note only.
- Do NOT push/PR unless instructed.

## Steps

### Step 1 (A): Put the mux socket in a user-private 0700 directory

Change `Master::establish` (`transport.rs:54-61`) to create the socket under a
per-user private dir instead of `/tmp`: prefer `$XDG_RUNTIME_DIR/ruxel/`
(already 0700 and user-owned) if set, else `~/.ruxel/cp/` created with 0700.
Create the dir (0700) if absent; keep the `pid + nanos` suffix for uniqueness
(and add more entropy per plan 022's needs if trivial). Preserve all existing
ssh args.

**Verify**: `cargo build -p ruxel-cli` → 0; the socket path now resolves under a
0700 dir (add a small unit test for the path-selection helper if you factor it:
`socket_dir()` returns the XDG/HOME path, not `/tmp`). The on-VM transport gate
(`RUXEL_TEST_SSH_DEST=... cargo test ... transport_gate -- --ignored`) still
connects — operator/fixture, note it.

### Step 2 (B): Re-verify the remote agent's hash before trusting it

In `ensure_agent`, after `test -x <remote_path>` succeeds (or after upload),
compute the remote file's hash (e.g. run `sha256sum`/`b3sum` over SSH, or fetch
and hash) and compare to the expected digest embedded in the path. On mismatch,
re-upload (or error). Keep the fast path (skip upload on hash hit) but gate the
*spawn* on a verified match. If adding a remote hash round-trip is deemed too
costly for every run, at minimum verify **once per process** and document the
tradeoff in a comment.

**Verify**: `cargo build -p ruxel-cli` → 0. This is transport behavior; the
on-VM gate confirms upload-skip still works. If a full re-hash is too invasive
for this plan, implement the comment + a `// TODO(SECURITY-08)` and **explicitly
defer** (note in the status row) rather than shipping a partial check that looks
complete.

### Step 3 (C): Redact `Debug` on secret-bearing types

Replace the `#[derive(Debug)]` on `Scope` (`engine.rs:162`) and `ScopeObject`
(`:204`) — and `VarValue` (`:154`) if it can hold a resolved secret — with
manual `impl Debug` that prints a redaction placeholder for the value-bearing
fields (e.g. `f.debug_struct("ScopeObject").field("memo", &"<redacted>").finish()`).
Keep the struct otherwise debuggable (field names, not values). Do **not** derive
`Debug` on the memo map.

**Verify**: `cargo build -p ruxel-core` → 0; `grep -n "derive(Debug)" crates/ruxel-core/src/engine.rs`
→ the secret-bearing types no longer derive it; a unit test asserting
`format!("{:?}", scope_object_with_a_value)` does **not** contain the value
string (use a synthetic non-secret value in the test). `cargo nextest run -p
ruxel-core` → pass.

### Step 4 (D): Surface the committed capture credential to the operator

Do **not** read or reproduce the value. In the PR description and a new
`plans/README.md` "Findings considered and rejected"/follow-up note, record:
"`tools/oracle/captures/pg-bless.jsonl` contains a 9-character literal password
for the PG `looker` role that is not a `dry-secret` fake. If it corresponds to a
real role password, rotate it and re-capture with `RUXEL_DRY_SECRETS=1` so the
bless renders a fake; also review whether committing real `op://` item paths and
the GCP key filename in the captures is intended." Leave the decision to the
operator (scrubbing history needs force-push authorization).

**Verify**: the note exists in the PR/README; **no** capture file was modified
by this plan (`git status` shows no `tools/oracle/captures/` changes).

### Step 5: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- A: `socket_dir()` selection unit test (not `/tmp`).
- C: `Debug` redaction test (synthetic value not leaked).
- B: covered by the on-VM transport gate (upload-skip + spawn still work); unit
  test the hash-compare helper if factored.
- D: no test (report only); assert no capture file changed.

## Done criteria

ALL must hold:

- [ ] The mux socket resolves under a 0700 user-private dir, not `/tmp`
- [ ] The agent's remote hash is re-verified before spawn (or SECURITY-08 explicitly deferred with a TODO + status note)
- [ ] Secret-bearing `engine.rs` types no longer derive `Debug`; a test proves values aren't printed
- [ ] The committed-capture credential is surfaced for operator rotation (no value reproduced; no capture file edited)
- [ ] `cargo nextest run` green; clippy/fmt clean
- [ ] `plans/README.md` row for 015 updated + the D follow-up note recorded

## STOP conditions

Stop and report if:
- Moving the socket off `/tmp` breaks the on-VM transport gate (path length
  limits for unix sockets are ~104 chars — a long `$HOME` could overflow; if so,
  fall back to a short `/run/user/<uid>/ruxel/` path and report).
- Re-hashing the remote agent adds unacceptable per-run latency — defer
  SECURITY-08 explicitly rather than shipping a partial check.
- You are tempted to open `pg-bless.jsonl` to "check" the value — **don't**; the
  finding stands on the shape alone. Report and let the operator decide.

## Maintenance notes

- The unix-socket path length limit (~104 bytes) is a real constraint — keep the
  private-dir path short.
- Reviewer: confirm no capture file was modified and the PR note does not quote
  the credential.
- Plan 022 (multi-host parallelism) will revisit the socket name for
  cross-host uniqueness — this plan only makes the *directory* private.

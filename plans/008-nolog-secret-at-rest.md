# Plan 008: Honor `no_log` agent-side; stop persisting secrets at rest

> **Executor instructions**: Follow step by step; run every verify command.
> Honor STOP conditions. Update this plan's row in `plans/README.md` when done.
> This is a **security** plan: be precise, and never write a secret value into
> any file, test, or commit — reference credential *types* and locations only.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/main.rs crates/ruxel-agent/src/ledger.rs crates/ruxel-agent/src/modules/copy.rs crates/ruxel/src/scheduler.rs`
> If any changed, re-verify the excerpts; on mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW-MED (a `no_log` cacheable task stops being ledger-cached — it
  will re-check every run; that is correct and acceptable)
- **Depends on**: 005 (ledger test harness)
- **Category**: security
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

`ARCHITECTURE.md §8` states `no_log` "redacts protocol logging, diffs, JSON
output, and ledger identity hashing." The **controller** honors it for
printed output and diffs (`scheduler.rs`), but the **agent never reads
`no_log`**: it records every non-check task's full result JSON into
`/var/lib/ruxel/ledger/ledger.json`, and for `copy`/`template` under `--diff`
that result contains the **full plaintext before/after file content**. So a
`no_log: true` `template` task that renders a secret (e.g. a DB password from a
1Password lookup) into a config file, run with `--diff`, writes that secret in
cleartext into the target's ledger — and the ledger file is created
world-readable (mode 0644). Two compounding issues: the ledger file
permissions, and the ledger **key** being an unkeyed `blake3` of rendered
params (which for content modules includes the secret-bearing file content),
making a weak embedded secret offline-guessable by anyone who can read the
file. This plan closes the at-rest exposure. Any secret already written to a
real host's ledger by the current code should be considered exposed —
recommend the operator rotate affected credentials and wipe stale ledgers
(`ruxel ... --no-cache` regenerates).

## Current state

**Agent ignores `no_log`:**
- `crates/ruxel-proto/proto/ruxel.proto:69` defines `RenderedTask.no_log`
  (field 7); the controller sets it (`crates/ruxel/src/scheduler.rs:598`
  `no_log: task.no_log`).
- `crates/ruxel-agent/src/main.rs:185-296` `execute_task` receives `task:
  &v1::RenderedTask` (so `task.no_log` is available) but **never reads it**.
- `crates/ruxel-agent/src/main.rs:277-285` records unconditionally:
  ```rust
  if !task_check_mode {
      ledger.record(&iteration.ledger_key, &task.module, &params,
                    outcome.status, &outcome.result);
  }
  ```
- `crates/ruxel-agent/src/modules/copy.rs:31-35` embeds full content under
  `--diff`:
  ```rust
  if ctx.diff_mode {
      let before = String::from_utf8_lossy(&current);
      result["diff"] = json!(super::unified_diff(&before, content));
  }
  ```
  (`copy::run` also backs `template`, per `modules/mod.rs:114`.) `send_result`
  (`main.rs:298-325`) additionally puts `result["diff"]` into the wire
  `TaskResult.diff`.

**Ledger file permissions:**
- `crates/ruxel-agent/src/ledger.rs:90-103` `flush` writes via `std::fs::write`
  then `rename` with **no `set_permissions`** → default umask (0644,
  world-readable). `main.rs:56-57` and `ledger.rs:95` create the dirs with
  default 0755.

**Unkeyed ledger identity:**
- `crates/ruxel/src/scheduler.rs:566-581` builds `ledger_key` as a plain
  `blake3` over `playbook_dir ‖ module ‖ label ‖ item_label ‖ params_bytes ‖
  free_form`. `params_bytes = serde_json::to_vec(&params)` includes rendered
  secret-bearing content. `ARCHITECTURE.md:183-186` specifies replacing
  secret-derived values with `HMAC(host_ledger_key, value)`; that substitution
  does not exist.

**Dead redaction helper:**
- `crates/ruxel-core/src/task_eval.rs:123-141` `censored_result` is tested
  (goldens E12/E13) but has **no production caller** (grep). The scheduler
  instead blanks the display string (`scheduler.rs:652-658`) and suppresses
  diffs (`:663-672`) for `no_log`, but the **registered variable stays
  uncensored by design** (Ansible parity — later tasks read the real data; keep
  that).

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Ledger tests | `cargo nextest run -p ruxel-agent ledger` | pass |
| Agent build | `cargo build -p ruxel-agent` | exit 0 |
| Scheduler build | `cargo build -p ruxel-cli` | exit 0 |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |
| Confirm no_log now read | `grep -n "no_log" crates/ruxel-agent/src/main.rs` | matches after the fix |

## Scope

**In scope**:
- `crates/ruxel-agent/src/main.rs` — thread `task.no_log` into the record/diff
  decision.
- `crates/ruxel-agent/src/modules/mod.rs` — add `no_log` to `ExecContext` (or
  pass a flag) so content modules can skip embedding the diff.
- `crates/ruxel-agent/src/modules/copy.rs` — do not embed plaintext diff for
  `no_log` tasks.
- `crates/ruxel-agent/src/ledger.rs` — set 0600 on the ledger file, 0700 on its
  dir; skip recording result payloads for `no_log` tasks (or store a redacted
  result).
- `crates/ruxel/src/scheduler.rs` — the `ledger_key` derivation (Step 4, the
  HMAC/keying decision).
- Tests in `ledger.rs` / a new agent test.

**Out of scope**:
- The controller-side printed-output redaction (`scheduler.rs:652-672`) — it
  already works; don't regress it.
- Keeping the registered variable **uncensored** — that is deliberate Ansible
  parity (goldens E12/E13); do **not** censor the register payload the
  controller uses for later templating.
- Rotating real secrets / wiping production ledgers — operator action; document
  it in the PR, do not attempt any host contact.

## Git workflow

- Branch: `advisor/008-nolog-at-rest`
- Commit in slices: (a) agent honors no_log for ledger+diff; (b) ledger file
  perms; (c) ledger key keying. Or one
  `fix(security): honor no_log at rest — skip ledger/diff for secret tasks, 0600 ledger`.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Thread `no_log` into the agent execution context

Add a `no_log: bool` field to `crates/ruxel-agent/src/modules/mod.rs`
`ExecContext` (`:34-42`). Set it in `main.rs` where `ExecContext` is built
(`:260-273`) from `task.no_log`.

**Verify**: `cargo build -p ruxel-agent` → 0.

### Step 2: Skip diff embedding for `no_log` content tasks

In `crates/ruxel-agent/src/modules/copy.rs:31-35`, gate the diff embedding on
`!ctx.no_log` (and mirror in any other content module that embeds `diff` — grep
`result\["diff"\]` across `crates/ruxel-agent/src/modules/`; today only
`copy.rs` does). When `no_log`, either omit `diff` entirely or set it to a fixed
redaction marker string (no content). `send_result` then carries no plaintext.

**Verify**: `grep -rn 'result\["diff"\]' crates/ruxel-agent/src/modules/` shows
each site now guards on `no_log`; `cargo build -p ruxel-agent` → 0.

### Step 3: Do not persist `no_log` results in the ledger; set 0600 perms

- In `main.rs:277-285`, skip `ledger.record(...)` when `task.no_log` (a no_log
  cacheable task will re-check every run — correct, since we must not store its
  fingerprint-associated result). Alternatively pass `no_log` into
  `ledger.record` and store a redacted result with **no** `diff`/content — but
  the simplest safe rule is: **do not cache no_log tasks**. Implement the skip.
- In `ledger.rs::flush` (`:90-103`), after writing the tmp file and before/at
  rename, set the file mode to `0600` (`std::fs::set_permissions(&tmp,
  Permissions::from_mode(0o600))` before rename; `#[cfg(unix)]`). Create the
  `ledger/` dir with `0700` (set perms after `create_dir_all` at `:94-95`, and
  the `/var/lib/ruxel` dir at `main.rs:56-57`).

**Verify**: add ledger test `flush_sets_0600` (`#[cfg(unix)]`): record+flush,
then assert `symlink_metadata(ledger.json).permissions().mode() & 0o777 ==
0o600`. Add `no_log_task_not_cached`: after `execute_task` on a `no_log` copy
(or a direct `record` call with a no_log flag if you route it that way),
`cached_ok(key)` is `None`. `cargo nextest run -p ruxel-agent ledger` → pass.

### Step 4: Key the ledger identity so stored keys aren't guessable secret hashes

The ledger key is stored as a JSON object key in the (now 0600, but still
on-disk) ledger. For content modules it currently equals `blake3(context ‖
secret-bearing content)`. Implement **one** of:
- **(Preferred, matches §6)** Derive a per-host `host_ledger_key` (32 random
  bytes) stored at `/var/lib/ruxel/ledger/key` mode 0600, created on first use.
  Replace the plain `blake3` in `scheduler.rs:566-581` with a keyed hash — but
  note the key lives on the **agent**, while the key is currently computed on
  the **controller** (`scheduler.rs`). That crosses the wire boundary. The
  clean design: move ledger-key computation to the **agent** (it already
  receives `params_json`), keying with its local `host_ledger_key`, and have
  the controller send an unkeyed identity tuple. This is a larger change —
  scope it carefully.
- **(Minimal, if the above is too large for this plan)** Keep the controller
  computing the key but HMAC the whole params blob with a value that is at least
  not the plaintext — however, without a host secret on the controller this
  gains little. **If the preferred design doesn't fit here, do NOT ship a
  fake-security half-measure**: instead, rely on Steps 1–3 (no secret content
  is stored for no_log tasks, and the file is 0600) and leave a
  `// TODO(SECURITY-04): key ledger identity with a per-host secret
  (ARCHITECTURE §6)` at `scheduler.rs:567`, and record SECURITY-04 as a
  follow-up in `plans/README.md`. Steps 1–3 already remove the *secret content*
  from the ledger; Step 4's keying is defense-in-depth for the identity hash.

Decide based on effort budget; if you defer Step 4, say so explicitly in the PR
and the status row.

**Verify**: if implemented, add a test that two runs with the same params
produce the same key (idempotence preserved) but the stored key is not a bare
hash of the content (i.e. a host key file exists and participates). If deferred,
the TODO is present and `plans/README.md` notes SECURITY-04 as open.

### Step 5: Resolve the dead `censored_result`

`task_eval::censored_result` is unused in production. Either wire it into the
controller's `no_log` output path (so `no_log` tasks show Ansible's explicit
`censored` marker instead of a blank line) **or** delete it and its test if the
blank-line behavior is intended. Prefer wiring it (closer to Ansible parity) —
but this is cosmetic; if it risks scope creep, delete it with a comment and note
the output-shape difference. Do not change the **registered variable**
(stays uncensored).

**Verify**: `grep -rn "censored_result" crates/` → either a production caller
exists, or the function and its test are both gone (no dead code).

### Step 6: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- `flush_sets_0600` (`#[cfg(unix)]`) — ledger file is not world-readable.
- `no_log_task_not_cached` — a `no_log` task leaves no ledger record.
- Agent-level (in `ledger.rs` tests or `protocol.rs`): a `no_log` `copy` under
  `--diff` produces a `TaskResult` whose `diff` is empty/redacted (no plaintext).
- Model on plan 005's harness. **Never** put a real secret in a test — use a
  synthetic string like `"test-content-not-a-secret"`.

## Done criteria

ALL must hold:

- [ ] `grep -n "no_log" crates/ruxel-agent/src/main.rs` → the agent reads it
- [ ] A `no_log` task is not recorded in the ledger; `no_log` content tasks embed no plaintext diff
- [ ] `ledger.json` is created mode 0600 and its dir 0700 (test proves it)
- [ ] Ledger key: either keyed with a per-host secret, OR Steps 1–3 landed with an explicit deferred `TODO(SECURITY-04)` and a README follow-up
- [ ] `censored_result` is either wired in or removed (no dead code)
- [ ] The registered variable remains uncensored (Ansible parity intact — goldens E12/E13 still pass)
- [ ] `cargo nextest run` green; clippy/fmt clean
- [ ] PR description recommends the operator rotate any secret previously written to a real host's ledger and wipe stale ledgers
- [ ] `plans/README.md` row for 008 updated

## STOP conditions

Stop and report if:
- Step 4's preferred keyed-identity design turns out to require moving ledger-key
  computation across the wire boundary in a way that risks the ledger fast path
  — defer Step 4 (explicitly), land Steps 1–3, and report.
- Removing/wiring `censored_result` breaks the E12/E13 goldens — those pin the
  **register** payload shape, which must stay uncensored; re-read the goldens
  and adjust only the *output-display* path.
- Any test would require a real credential — it must not; use synthetic strings.

## Maintenance notes

- The load-bearing invariant: **no secret value is ever written to the target
  disk except the managed file itself** (that write is the operator's intent).
  The ledger, diffs, and logs must never be a second copy. Any future cacheable
  content module must respect the `no_log` skip.
- Reviewer: verify the register payload used for later templating is still the
  real (uncensored) value — over-censoring would break dependent tasks.
- Follow-up: if Step 4 was deferred, SECURITY-04 (keyed ledger identity) remains
  open — track it in the README.

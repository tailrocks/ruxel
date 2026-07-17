# Plan 012: Fix module correctness bugs (lvol, replace, file, low-conf cluster)

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. Each
> sub-fix is independent — you may land them separately.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/modules/ crates/ruxel-core/src/modules.rs crates/ruxel-core/src/engine.rs`
> If any changed, re-verify the excerpts; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: M (several small independent fixes)
- **Risk**: LOW-MED per fix (noted inline)
- **Depends on**: none
- **Category**: bug (correctness)
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

A cluster of module-level divergences from Ansible, each a real correctness bug
on the workload's surface. The two HIGH-value ones: `lvol` **fails first-disk
provisioning** with `size: +100%FREE`, and `replace` **corrupts content**
containing `$`. The rest are lower-confidence "investigate" items found by
reading — each is a genuine divergence but its workload incidence is
unconfirmed (the private playbooks aren't in-repo), so each gets a targeted fix
+ test, and a STOP hatch if reality differs.

## Current state (per fix)

**A. CORRECTNESS-12 — `lvol` create with `+100%FREE` (MED confidence, HIGH
impact):** `crates/ruxel-agent/src/modules/lvol.rs:83-87`:
```rust
fn create(vg: &str, lv: &str, size: &str) -> Result<(), String> {
    let flag = if size.contains('%') { "-l" } else { "-L" };
    run_cmd("lvcreate", &[flag, size, "-n", lv, vg])   // e.g. lvcreate -l +100%FREE ...
}
```
`lvcreate -l` expects `100%FREE`, **not** `+100%FREE` — the `+` is an
`lvextend`/`lvresize` relative operator. `SEMANTICS.md §6` lists `size: +100%FREE`
in use. On a **fresh** disk (LV absent) this fails at `lvcreate`. The extend
path (`:31-49`) handles `+`; only `create` forwards it raw.

**B. CORRECTNESS-09 — `replace` regex `$` semantics (MED confidence):**
`crates/ruxel-agent/src/modules/replace.rs:17`:
```rust
let next = re.replace_all(&current, replacement).to_string();
```
`regex_lite`'s `replace_all` interprets `$name`/`${n}` in `replacement` as
capture references (like the `regex` crate). Ansible's `replace` uses Python
`re.sub`, where `$` is **literal** and backrefs are `\1`. A `replace:` value
containing `$` (env vars, systemd specifiers, prices, `$PATH`) is silently
mangled; Ansible-style `\1` backrefs are emitted literally. 3 `replace` sites.

**C. CORRECTNESS-13 — `file` with no `state` errors at runtime (MED confidence):**
`crates/ruxel-agent/src/modules/file.rs:13,67`:
```rust
let state = str_param(obj, "state").unwrap_or("file");   // default "file"
// ...
other => Err(format!("file: state {other:?} outside the closed surface")),  // "file" hits this
```
The parser only validates `state` when present, so a `file:` task that manages
only `owner`/`mode` on an existing path (Ansible's default `state: file`) parses
fine then **hard-fails at execution**. SEMANTICS §6 counts 46 `file` uses but
only 42 explicit states (directory 30 + absent 11 + link 1) — ~4 may omit
`state`.

**D. Low-confidence investigate cluster (each real, incidence unconfirmed):**
- **`command` swallows unclosed quotes** — `command.rs:80-88`: an unterminated
  `'`/`"` is consumed to EOL silently; Python `shlex.split` raises "No closing
  quotation". A malformed templated command mis-parses instead of failing.
- **`ansible.posix.sysctl` missing `sysctl_file` param** — `modules.rs:44-50`:
  the registry's `ansible.posix.sysctl` params are `["name","reload","state",
  "sysctl_set","value"]`, **no `sysctl_file`**, though `SEMANTICS.md §6` lists
  `sysctl_file` for that spelling and `sysctl.rs:26` reads it. A playbook using
  `ansible.posix.sysctl` + `sysctl_file` would hard-error at **parse**. (The
  "all 16 parse" claim suggests it may not be exercised, or the bare `sysctl`
  spelling carries it — verify which spelling the workload uses with sysctl_file.)
- **`filesystem` xfs `resizefs` is a no-op** — `filesystem.rs:65-71`: `grow`
  returns `Ok(false)` for xfs (relies on `lvol -r`). A standalone `filesystem:
  resizefs=yes` on xfs won't grow.
- **`iptables` append-only, no insert** — `iptables.rs:56-60`: always `-A`; no
  `action`/`rule_num` param, so an insert-ordered rule (the DOCKER-USER
  workaround SEMANTICS §6 mentions) can't be expressed and lands at the wrong
  position.
- **`blockinfile create: no` on a missing file errors** — `blockinfile.rs:19-23`:
  returns `Err` where Ansible returns unchanged.
- **CORRECTNESS-14 — failed lazy var is truthy in `when`** — `engine.rs`
  (auditor cites `:232-238` lazy render returns a `ScopeRenderError` object,
  `:405-415` `eval_expr_bool` errors only on `is_undefined()`, `:222-224` cycle
  yields a placeholder string). A play var that fails to render but is consumed
  in a condition silently evaluates true; a variable cycle renders a placeholder
  into a file instead of erroring. **Read these lines and confirm before acting.**

**Convention**: modules return `Result<Value, String>` with an Ansible-shaped
result dict. Inline `#[cfg(test)] mod tests` for pure logic (see
`command.rs:132`, `sysctl.rs:116`).

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Agent module tests | `cargo nextest run -p ruxel-agent` | pass |
| Core (engine/registry) tests | `cargo nextest run -p ruxel-core` | pass |
| Build | `cargo build --workspace` | exit 0 |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**: `crates/ruxel-agent/src/modules/{lvol,replace,file,command,
filesystem,blockinfile,sysctl,iptables}.rs`, `crates/ruxel-core/src/modules.rs`
(the sysctl param), and `crates/ruxel-core/src/engine.rs` (CORRECTNESS-14 only,
and only if confirmed).

**Out of scope**:
- Ledger probe changes (plan 006), no_log (008), pg (011).
- Adding modules/params beyond the closed surface — the sysctl_file fix aligns
  the registry to SEMANTICS, it does not add new surface.
- Anything requiring a live LVM/xfs/iptables system — those are on-VM gates.

## Git workflow

- Branch: `advisor/012-module-fixes`
- One commit per lettered fix (A–D); or group A/B/C as `fix(modules): lvol
  +100%FREE create, replace literal-$, file default state` and D as a second.
- Do NOT push/PR unless instructed.

## Steps

### Step A: `lvol` create strips the `+` prefix

In `lvol.rs::create` (`:83-87`), strip a leading `+` from `size` before passing
to `lvcreate` (the `+` is only meaningful to `lvextend`). Use
`size.trim_start_matches('+')` for the `-l`/`-L` argument. Keep the extend path
unchanged.

**Verify**: add a pure unit test for a small helper `lvcreate_size(size) ->
&str` (or test the arg vector construction) asserting `+100%FREE` → `100%FREE`
and `100%FREE` → `100%FREE` and `10G` → `10G`. `cargo nextest run -p
ruxel-agent lvol` → pass. (Live LVM behavior is an on-VM gate.)

### Step B: `replace` uses literal replacement semantics

Make `replace_all` treat the `replace:` value as a **literal** string (Ansible's
`re.sub` treats `$` as literal). Two options with `regex_lite`:
- If `regex_lite` exposes a `NoExpand`/literal replacer, use it.
- Otherwise, escape `$` → `$$` in `replacement` before `replace_all` (regex
  replacement syntax uses `$` for groups; doubling escapes it).
Confirm which `regex_lite` supports (read its docs/version in `Cargo.lock`).
**Decision on backrefs**: SEMANTICS lists `replace` as plain regex substitution;
if the workload uses **no** backrefs, literal is correct. If it uses Ansible
`\1` backrefs, translate `\1`→`$1` and still `$$`-escape literals. Prefer
literal-only and add a STOP if you find a `\`-backref in the workload.

**Verify**: unit test `replace_dollar_is_literal`: replacing a match with
`"$PATH"` yields literal `$PATH` in the output, not an empty/expanded string.
`cargo nextest run -p ruxel-agent replace` → pass.

### Step C: `file` default `state: file` applies attrs to an existing path

Add a `"file"` (default) arm in `file.rs`'s `match state` that: `symlink_metadata`s
the path; if it doesn't exist, error like Ansible ("file does not exist"); if it
exists, run `apply_attrs` (mode/owner/group) with no create. Do **not** create
the file (Ansible's `state: file` doesn't create; `touch` does, which the
workload doesn't use per SEMANTICS).

**Verify**: unit/integration test `file_no_state_applies_attrs`: on a scratch
file with a known mode, a `file` task (no `state`) with `mode: "0600"` changes
the mode and reports `changed`; on a **missing** path it errors (not "outside
the closed surface"). `cargo nextest run -p ruxel-agent file` → pass.

### Step D: Low-confidence cluster (fix the confirmed, guard the rest)

For each, **confirm against the code first**, then fix:
- **command unclosed quote**: in `shlex_split` (`command.rs:73-130`), track
  whether a quote opened and never closed; at end, if still inside a quote,
  return `Err("No closing quotation")` (match Python). Unit test:
  `shlex_split("echo 'unterminated")` → `Err`.
- **ansible.posix.sysctl `sysctl_file`**: add `"sysctl_file"` to the
  `ansible.posix.sysctl` params in `modules.rs:47-49` (aligning to SEMANTICS §6;
  the agent module already reads it). Verify the workload-parse gate still
  passes (`cargo test -p ruxel-core --test workload` with `RUXEL_WORKLOAD_DIR`
  set, if available; else confirm by reading SEMANTICS). **STOP** if adding it
  breaks a parse test — that would mean the bare `sysctl` spelling is the one
  used and the change is wrong.
- **filesystem xfs resizefs**: implement xfs grow via `xfs_growfs <mountpoint>`
  **only if** the module can determine the mountpoint; since the workload grows
  xfs after mount (per the code comment `:68-70`), the safest fix is to leave
  the no-op but return an honest result and add a comment — **do not** invent a
  mountpoint. Mark this one "confirmed-by-design, documented" unless the
  workload has a standalone xfs resize (STOP and report if so).
- **iptables insert**: the closed surface has no `action`/`rule_num` param, so
  insert can't be expressed — this is a **spec** question, not a code bug to fix
  unilaterally. Add a code comment noting append-only and that DOCKER-USER
  ordering relies on rule content, and leave a `// TODO(spec): iptables insert
  semantics if a rule_num appears`. **Do not** add a param (closed surface).
- **blockinfile create:no on missing file**: change `blockinfile.rs:19-23` so
  that `create: false` + missing file returns `changed: false` (unchanged),
  matching Ansible, instead of `Err`. Unit test with a non-existent path.
- **CORRECTNESS-14 (engine lazy-var truthy)**: read `engine.rs:222-238,405-415`.
  If confirmed, make `eval_expr_bool` treat a `ScopeRenderError` poison value as
  an error (propagate `EngineError`) rather than truthy, and make a variable
  cycle a hard error (this is where the dead `EngineError::VarCycle` from plan
  003 could be *used* instead of deleted — coordinate: if 003 hasn't run, wire
  `VarCycle` here; if 003 deleted it, re-add). This is the trickiest item —
  **STOP and report** if propagating the error surfaces failures in the
  render-parity goldens (`cargo nextest run -p ruxel-core render_parity`), since
  those pin the currently-tolerated behavior; the fix must not break byte-parity
  on the exercised corpus.

**Verify** (per item): the named unit test passes; `cargo nextest run
--workspace` shows no regressions; render-parity goldens still pass for the
engine change.

### Step E: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- A: `lvcreate` size normalization (`+100%FREE`→`100%FREE`).
- B: `replace` literal `$`.
- C: `file` default-state applies attrs / errors on missing.
- D: `shlex_split` unclosed-quote error; `blockinfile create:no` unchanged;
  `sysctl_file` param accepted (workload parse gate); engine lazy-var/cycle (only
  if confirmed and parity-safe).
Model on the inline test modules already in `command.rs`/`sysctl.rs`.

## Done criteria

ALL must hold:

- [x] `lvol` create strips `+` (unit test proves `+100%FREE`→`100%FREE`)
- [x] `replace` treats `$` literally (unit test proves `$PATH` survives)
- [x] `file` with no `state` applies attrs to an existing path / errors on missing (not "outside the closed surface")
- [x] `shlex_split` errors on an unclosed quote; `blockinfile create:no` on a missing file is unchanged (not Err)
- [x] `ansible.posix.sysctl` registry includes `sysctl_file` (and the workload parse gate still passes) OR a STOP was reported explaining why not
- [x] CORRECTNESS-14 either fixed with render-parity goldens still green, OR explicitly deferred with a reason
- [x] `cargo nextest run` green; clippy/fmt clean
- [x] `plans/README.md` row for 012 updated

## STOP conditions

Stop and report if:
- Adding `sysctl_file` to `ansible.posix.sysctl` breaks the workload parse gate
  (means the bare `sysctl` spelling is the sysctl_file user — revert and report).
- The `replace` workload values contain `\`-backrefs (literal-only would break
  them) — report and implement the `\n`→`$n` translation instead.
- Fixing CORRECTNESS-14 breaks any render-parity golden — the engine change is
  too aggressive; narrow it or defer, and report.
- `xfs_growfs` would need a mountpoint you can't safely derive — leave the no-op
  documented and report.

## Maintenance notes

- A/B/C are the real bugs; D is a mix of small fixes and spec questions. The
  iptables-insert and xfs-resize items are partly **spec** decisions — flag them
  for the operator rather than expanding the closed surface unilaterally.
- Reviewer: B (replace `$`) is the sneakiest — a wrong escaping direction just
  moves the corruption. Verify with a concrete `$`-bearing and (if applicable)
  backref-bearing case.
- Live LVM/xfs/iptables behavior is confirmed only by the on-VM bless-gate;
  these unit tests cover the pure argument/parse logic.

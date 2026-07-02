# Plan 019: DX tooling — justfile, release story, oracle Python pin, renovate

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- mise.toml renovate.json rust-toolchain.toml tools/oracle/pyproject.toml README.md AGENTS.md`
> If any changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Small quality-of-life gaps for an agent-driven, single-operator repo: (1) the
four quality gates are prose instructions in `AGENTS.md`/`README.md` with no
`just`/pre-commit wrapper, so each agent must remember the exact commands (a
past `clippy` slip-through is recorded in `GOAL.md:390`), and there's no
unused-dep linter (which let plan 003's dead deps linger). (2) There's **no
release/versioning story** — no git tags, no `CHANGELOG`, everything is
`0.1.0`, and the operator installs by local `cargo build`, so "which `ruxel` is
deployed?" is unanswerable (a real gap for a tool that runs against production).
(3) The oracle's Python interpreter floats (venv is 3.13, lockfile resolved
cp312 wheels; `uv` pinned "latest"), a hermeticity gap for a parity oracle whose
whole job is byte-reproducing pinned Ansible. (4) Renovate doesn't cover the
`rust-toolchain.toml` channel pin (it'll silently go stale).

## Current state

- Quality gates: `AGENTS.md:36-37` and `README.md:60-66` list `cargo fmt --all
  --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. No
  `justfile`/`Makefile`/`.pre-commit-config.yaml`/`.editorconfig` (verified
  absent). No `cargo-machete`/`cargo-udeps`.
- Versioning: `git tag -l` → empty; no `CHANGELOG.md`; workspace `version =
  "0.1.0"` (`Cargo.toml:11`, all crates `version.workspace = true`). Install path
  is local build (`RESTORE.md:124-126`); crates.io publishing is off by design
  (`crates/ruxel/Cargo.toml:1-3`, bare name taken).
- Oracle: `tools/oracle/pyproject.toml:5` `requires-python = ">=3.12"` (floor,
  not pin); the committed `uv.lock` resolved cp312 wheels but the on-disk venv is
  3.13 (`.venv/lib/python3.13/...`). `mise.toml:4` pins `uv = "latest"` (the one
  unpinned tool); zig/cargo-* are pinned exactly. `.venv`/`galaxy`/`__pycache__`
  are correctly gitignored.
- Renovate: `renovate.json` extends `config:best-practices` + `lockFileMaintenance`,
  no `customManagers`. Renovate's built-in `cargo` manager updates
  `Cargo.toml`/`Cargo.lock` but not the `channel` field of `rust-toolchain.toml`
  (`:2`, `1.96.0` — currently *fresh*, released 2026-05-25; this is about the
  *update mechanism*, not present staleness). Whether the `mise` manager covers
  `mise.toml` tool pins is uncertain — verify.

**Convention**: mise for tool pinning; SHA-pinned GitHub Actions; conventional
commits.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Just (after install) | `just check` | runs fmt+clippy+test |
| Machete (if added) | `cargo machete` | reports unused deps (should be none post-003) |
| Validate renovate JSON | `python3 -c "import json;json.load(open('renovate.json'))"` | exit 0 |
| Oracle python | `cd tools/oracle && uv run python --version` | the pinned version |

## Scope

**In scope**: `justfile` (new), `.editorconfig` (new), `renovate.json`
(customManager for rust-toolchain), `mise.toml` (pin `uv`), `tools/oracle/`
(add `.python-version` / pin), `CHANGELOG.md` (new, minimal), README/AGENTS
(point at `just check`). Optionally a release workflow (Step 4 — scope-gated).

**Out of scope**:
- Bumping the rust toolchain or any dep version (only the *mechanism*).
- Publishing to crates.io (off by design).
- CI cache/deny changes — plan 004.
- Adding pre-commit as a hard requirement (offer it; don't force a hook that
  blocks the operator).

## Git workflow

- Branch: `advisor/019-dx`
- Commit per item; `chore(dx): justfile + editorconfig + renovate/oracle pins`.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Add a `justfile` wrapping the quality gates + `.editorconfig`

Create a `justfile` at repo root with recipes:
- `check`: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo nextest run` (or `cargo test`), and (once added) `cargo
  machete`.
- `fmt`: `cargo fmt --all`
- `agent`: the musl cross-build (`mise exec -- cargo zigbuild --target
  x86_64-unknown-linux-musl -p ruxel-agent --release`) — copy the exact command
  from `RESTORE.md:124-126`.
Add `cargo-machete` to `mise.toml` tools (pinned) so `cargo machete` works.
Add a minimal `.editorconfig` (Rust: 4-space indent, LF, trim trailing ws).
Point `AGENTS.md`/`README.md` "quality gates" at `just check`.

**Verify**: `just check` runs all gates and exits 0 on a clean tree;
`cargo machete` reports no unused deps (assuming plan 003 landed).

### Step 2: Pin the oracle Python interpreter and `uv`

Add `tools/oracle/.python-version` set to the interpreter the controller runs
(match the `uv.lock`'s resolved CPython — determine it; the lock resolved cp312
wheels, so `3.12` is the likely pin, but confirm against what the operator's
real Ansible controller uses). Pin `uv` in `mise.toml` to an explicit version
instead of `latest`. Regenerate `uv.lock` under the pinned interpreter so the
venv and lock agree (`cd tools/oracle && uv lock`), or note that the operator
must re-run `uv sync` under the pinned Python.

**Verify**: `cd tools/oracle && uv run python --version` reports the pinned
version; `git diff tools/oracle/` shows the `.python-version` and (if
regenerated) `uv.lock` update. **Do not** change the pinned `ansible-core 2.21`
version — that must match production.

### Step 3: Add a renovate custom manager for `rust-toolchain.toml` + verify mise coverage

Add a `customManagers` regex entry to `renovate.json` that matches
`channel = "x.y.z"` in `rust-toolchain.toml` and treats it as a rust release
version, so Renovate opens bump PRs. Verify (via the Renovate docs or a
`renovate --dry-run` if available to the operator) whether the built-in `mise`
manager already covers `mise.toml` tool pins; if not, add regex managers for the
`cargo:*`/`zig`/`uv` pins too. Keep the existing `config:best-practices` +
`lockFileMaintenance`.

**Verify**: `renovate.json` still parses as JSON; the customManager regex
matches the `rust-toolchain.toml` line (test the regex with a quick
`python3 -c "import re; print(re.search(r'...', open('rust-toolchain.toml').read()))"`).

### Step 4 (scope-gated): Minimal release/versioning story

Add a `CHANGELOG.md` (Keep-a-Changelog style, an `Unreleased` section). Document
in the README a lightweight release ritual: annotated git tag on milestone +
`--version` already embeds the crate version (clap `version` at
`crates/ruxel/src/main.rs:9`). **Optional** (only if the operator wants
distribution): a SHA-pinned release workflow building `ruxel` (host) and
`ruxel-agent` (musl) as GitHub Release assets — the repo already proved the
deb-shipping pattern for `holla`, but a single-operator pilot can pin by
`git checkout <sha> && cargo build`. **Do not build the release workflow
without operator confirmation** — leave it as a documented option and a
`CHANGELOG.md` + tagging note. Note that `--version` currently prints `0.1.0`;
suggest the operator bump on the first tagged milestone.

**Verify**: `CHANGELOG.md` exists; `cargo run -p ruxel-cli -- --version` prints
the version (proves provenance handle exists); no release workflow added unless
confirmed.

### Step 5: Full gates

**Verify**: `just check` → 0 (or the individual `cargo fmt/clippy/nextest`).

## Test plan

No product code; verification is running `just check`, `cargo machete`, the
renovate JSON parse, and the oracle python version. Confirm `cargo run -- --version`
prints a version string (provenance).

## Done criteria

ALL must hold:

- [ ] `just check` runs fmt+clippy+tests(+machete) and exits 0; `.editorconfig` exists
- [ ] `cargo-machete` is pinned in mise and reports no unused deps
- [ ] The oracle has a pinned `.python-version`; `uv` is pinned (not "latest"); `ansible-core 2.21` is unchanged
- [ ] `renovate.json` has a custom manager for `rust-toolchain.toml` and parses as valid JSON
- [ ] `CHANGELOG.md` exists; README/AGENTS point at `just check`; no release workflow added without operator confirmation
- [ ] `plans/README.md` row for 019 updated

## STOP conditions

Stop and report if:
- The correct oracle Python pin is ambiguous (venv 3.13 vs lock cp312) and you
  can't determine what the operator's real Ansible controller uses — pin to the
  `uv.lock`'s resolved version (cp312 → 3.12) and note the assumption for the
  operator to confirm.
- Adding `cargo-machete` surfaces unused deps that plan 003 didn't remove —
  report them (they may be legitimately-conditional deps).
- The renovate custom-manager regex is uncertain to match — leave it out and
  report rather than shipping a regex that silently matches nothing.

## Maintenance notes

- The oracle Python pin protects the parity oracle's hermeticity — if it ever
  drifts again, blessed captures could be regenerated under a different CPython
  than they were pinned on.
- Reviewer: confirm `ansible-core 2.21` was **not** touched (it must match the
  production controller exactly).
- Release automation is deliberately deferred to the operator; revisit if
  distribution/multi-machine install becomes a goal (the holla-apt deb pattern
  is the proven template).

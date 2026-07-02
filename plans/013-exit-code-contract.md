# Plan 013: Make parse/usage errors exit 2 per the documented contract

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel/src/main.rs crates/ruxel/src/commands/apply.rs crates/ruxel/src/commands/plan.rs`
> If any changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug (contract)
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

`ARCHITECTURE.md §7` and `SEMANTICS.md §4` define the exit-code contract: **0**
success (regardless of changed), **1** any host failed, **2** usage/parse
error. Today a playbook or inventory **parse error exits 1** (it propagates as
an `anyhow::Error` out of `main() -> Result<()>`, which prints and exits 1),
identical to a host failure. A script or CI keying on exit codes cannot
distinguish "a host failed" (retryable) from "the playbook didn't parse"
(fix-the-file). clap's own usage errors already exit 2, so the contract is
half-honored and inconsistent. The fix maps parse/inventory/compile errors to
exit 2.

## Current state

- `crates/ruxel/src/main.rs:25-31`:
  ```rust
  fn main() -> Result<()> {
      let cli = Cli::parse();          // clap usage errors → exit 2 (clap default)
      match cli.command {
          Command::Plan(args) => commands::plan::execute(args),
          Command::Apply(args) => commands::apply::execute(args),
      }                                 // any Err → anyhow prints, exit 1
  }
  ```
- Parse/inventory error sites (all currently produce an `anyhow::Error` → exit 1):
  - `crates/ruxel/src/commands/apply.rs:71` `Inventory::parse(&inv_content)?`
  - `apply.rs:79` `ruxel_core::playbook::parse(&pb_name, &pb_content)?`
  - `crates/ruxel/src/commands/plan.rs:40` inventory parse, `:49` playbook parse,
    `:54` `compiler::compile(...)?`
- Host-failure exit is already correct: `apply.rs:195-197`
  `if any_failed { std::process::exit(1); }`.
- The typed error types available to distinguish parse errors:
  - `ruxel_core::playbook` parse returns a typed error (read `playbook.rs` for
    its error type name — likely a `thiserror` enum).
  - `ruxel_core::inventory::Inventory::parse` returns a typed error (read
    `inventory.rs`).
  - `ruxel_core::compiler::CompileError` (`compiler.rs:74-88`).
  These are wrapped by `?` into `anyhow::Error`; `anyhow` preserves the source
  so they can be `downcast_ref`'d.

**Convention**: `anyhow` at the CLI boundary; `thiserror` typed errors in
`ruxel-core`. Rust's `std::process::ExitCode` is the idiomatic way to return a
specific code from `main`.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Build | `cargo build -p ruxel-cli` | exit 0 |
| CLI tests | `cargo nextest run -p ruxel-cli` | pass |
| Manual: bad playbook | `cargo run -p ruxel-cli -- plan -i /dev/null nonexistent-or-broken.yml; echo $?` | exit **2** after fix |
| Manual: usage error | `cargo run -p ruxel-cli -- plan; echo $?` | exit 2 (clap, already) |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- `crates/ruxel/src/main.rs` — return `ExitCode`; map parse/compile errors to 2.
- Possibly `crates/ruxel/src/commands/{apply,plan}.rs` — if you choose to have
  `execute` return a typed error instead of `anyhow` (optional; downcasting in
  `main` avoids touching them).

**Out of scope**:
- Host-failure exit (already 1) — leave it.
- `--detailed-exitcode` (terraform-style 0/2) — that's plan 023, a different
  flag; do not conflate.
- Changing what counts as a parse error.

## Git workflow

- Branch: `advisor/013-exit-codes`
- One commit: `fix(cli): parse/usage errors exit 2 per the documented contract`
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Return `ExitCode` from `main` and map parse errors to 2

Change `main` to `fn main() -> std::process::ExitCode`. Run the command; on
`Ok(())` return `ExitCode::SUCCESS`. On `Err(e)`, print the error (preserve the
current `anyhow` formatting — e.g. `eprintln!("{e:?}")` or the `{e:#}` chain)
and inspect `e` with `downcast_ref` for the parse/inventory/compile error types:
if it is one of those, `return ExitCode::from(2)`; otherwise return
`ExitCode::from(1)`.

Concretely (adjust type names to the actual ones you find in `playbook.rs` /
`inventory.rs`):
```rust
fn main() -> std::process::ExitCode {
    let cli = Cli::parse(); // clap exits 2 on usage errors itself
    let result = match cli.command {
        Command::Plan(a) => commands::plan::execute(a),
        Command::Apply(a) => commands::apply::execute(a),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            if is_parse_error(&e) { ExitCode::from(2) } else { ExitCode::from(1) }
        }
    }
}
```
where `is_parse_error` checks `e.downcast_ref::<ruxel_core::playbook::ParseError>()`,
`::<ruxel_core::inventory::…>()`, and `::<ruxel_core::compiler::CompileError>()`
(use the real names). Read the three modules to get the exact types and whether
they're re-exported.

**Verify**: `cargo build -p ruxel-cli` → 0. Manual: feed a **malformed** YAML
playbook and confirm `echo $?` prints `2`; feed a valid playbook against an
unreachable/failing setup and confirm host-failure still yields `1` (or, if you
can't reach a host, reason from `apply.rs:196` that it stays 1).

### Step 2: Confirm host-failure and success codes unchanged

`apply.rs:195-197` already `exit(1)` on `any_failed`. Ensure your `main` change
doesn't override that (a `std::process::exit` inside `run` bypasses the
`ExitCode` return — that's fine; it's an explicit 1). Success returns 0.

**Verify**: `cargo nextest run -p ruxel-cli` → the existing CLI-parse tests
(`main.rs:33-93`) still pass (they test clap parsing, unaffected).

### Step 3: Add a test for the parse-error exit path

Because `main` calls `std::process::exit`/returns `ExitCode`, unit-testing the
exact code is awkward; instead test the classifier. Extract `is_parse_error`
(or the mapping) as a small pure fn and unit-test it: an `anyhow::Error` built
from a `CompileError`/parse error maps to 2; a generic `anyhow::anyhow!("host
down")` maps to 1. Put the test in `main.rs`'s `#[cfg(test)] mod tests`.

**Verify**: `cargo nextest run -p ruxel-cli` → the new classifier test passes.

### Step 4: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D
warnings` → 0; `cargo nextest run` → green.

## Test plan

- Unit: `is_parse_error` maps parse/inventory/compile errors → 2, others → 1.
- Manual (documented in the PR): a broken playbook → exit 2; a usage error →
  exit 2 (clap); a successful plan → 0.
- Existing `main.rs` clap tests must still pass.

## Done criteria

ALL must hold:

- [ ] `main` returns `ExitCode`; a playbook/inventory/compile parse error exits **2**
- [ ] Host failure still exits **1**; success exits **0**; clap usage errors exit **2**
- [ ] A unit test pins the error→code classifier
- [ ] `cargo nextest run` green; clippy/fmt clean
- [ ] `plans/README.md` row for 013 updated

## STOP conditions

Stop and report if:
- The parse error types aren't reachable/downcastable from `ruxel-cli` (e.g.
  they're private) — you may need a small `pub` re-export in `ruxel-core`;
  report the change before making a public API wider than necessary.
- Mapping to `ExitCode` conflicts with the `anyhow`-formatted error output the
  operator relies on — preserve the human-readable message; only the code changes.

## Maintenance notes

- Reviewer: verify the error *message* is still printed (don't swallow it while
  mapping the code).
- Plan 023 adds `--detailed-exitcode` (0=converged / 2=changes-needed). That is a
  **different** meaning of "2" gated behind an opt-in flag — keep the two
  distinct so the default contract (2=parse error) is unambiguous when the flag
  is absent.

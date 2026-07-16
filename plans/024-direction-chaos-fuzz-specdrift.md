# Plan 024: Chaos/fuzz hardening + spec-drift watch (spike + build)

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. This plan
> is part **spike** (design + prototype the chaos harness) and part **build**
> (the spec-drift extractor). If chaos tests surface a real re-entrancy bug,
> STOP and report it as a separate finding — don't fold the fix in here.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-core/src/playbook.rs crates/ruxel-proto/src/frame.rs crates/ruxel-agent/tests/ crates/ruxel-core/src/modules.rs docs/PLAN.md`
> If any changed, re-verify excerpts; on mismatch, STOP.
> Best after plans 009 (scheduler seam) and 017 (protocol tests) land.

## Status

- **Priority**: P2
- **Effort**: M (fuzz/property ~1 day; chaos harness multi-day; extractor ~1 day)
- **Risk**: LOW to build (test/tooling only); may **surface** real bugs — the point
- **Depends on**: 009, 017
- **Category**: direction (spike + tooling) — pre-production-contact hardening
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Two of the closed spec's own safety mechanisms are promised but absent:

1. **M5 chaos + fuzz/property hardening** (`PLAN.md:140-145`): the M5 gate is
   "no protocol state leaves a target unrecoverable," and `ARCHITECTURE.md §8`
   *asserts* re-entrancy / flock / "agent finishes the in-flight task" — but
   **nothing exercises it**. The scariest M6 failure is a controller Ctrl-C or
   link drop mid-apply leaving a half-written module action on a live 28 TiB
   ClickHouse / TB-scale Postgres host. This is the single gate that makes
   production contact defensible, and it's untested. Property tests on the
   parser/renderer also protect the "unknown = hard error" invariant the whole
   closed-spec model rests on.
2. **Spec-drift watch** (`PLAN.md:158-162`, README item 7): claimed to "live in
   `tools/spec-extract/` and run in CI" — it **doesn't exist**. This is the
   closed spec's *enforcement* mechanism. The workload is a live daily-driver
   (`SKEPTIC.md` shows setup-sentry/setup-delorean change several times a week);
   without the watch, a workload edit introducing a new module/param/value
   silently outruns SEMANTICS and the implementation (ruxel fails safe at
   runtime, but the operator discovers it mid-run and the normative doc drifts).

## Current state

- No `proptest`/`arbitrary`/`cargo-fuzz` in the tree (grep; clean `Cargo.lock`).
  The parser is `crates/ruxel-core/src/playbook.rs` (621 lines, the closed-spec
  trust boundary); the protocol framing is `crates/ruxel-proto/src/frame.rs`
  (64 MiB cap). The re-entrancy claims: agent flock (`main.rs:59-82`), "finishes
  in-flight task then EOF" (`main.rs:104-115` — plan 007 adds flush there),
  `kill_on_drop` (`transport.rs:230`). The only disconnect test today is the
  single kill-9 case in `protocol.rs`.
- `tools/` contains only `fixtures/` and `oracle/` — no `spec-extract`. The
  closed surface is `crates/ruxel-core/src/modules.rs` `MODULES` (36 surfaces
  with `params` + `literal_enums`). The workload lives in the **private**
  `ChainArgos/ansible-configs` repo (referenced via `RUXEL_WORKLOAD_DIR`).

**Convention**: tests are std + inline `#[cfg(test)]`. Adding `proptest` as a
**dev-dependency** (to `ruxel-core`/`ruxel-proto` `[dev-dependencies]`) is
acceptable (it doesn't bloat the shipped binaries). The agent stays lean; fuzz
the parser/frame in the crates that own them.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Property tests | `cargo nextest run -p ruxel-core proptest` (or the test name) | pass |
| Frame fuzz | `cargo nextest run -p ruxel-proto` | pass |
| Extractor (once built) | `cargo run -p ruxel-spec-extract -- <workload-dir>` OR a script | reports uncovered module/param/value |
| Chaos (on-VM) | operator/fixture harness | rerun converges after injected drop |
| Clippy/fmt | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**:
- Property/fuzz tests for `playbook::parse` and `frame::read_frame` (dev-dep
  `proptest`).
- A chaos harness (design + prototype) that injects disconnects at protocol
  states against a **fixture** (operator/fixture only, never production) and
  asserts rerun converges + the flock releases.
- A spec-drift extractor (`tools/spec-extract/` script or a small Rust tool)
  that diffs the workload's module/param/value triples against the closed
  registry.

**Out of scope**:
- Fixing any re-entrancy bug the chaos tests find — report it as a separate
  finding/plan.
- Wiring the extractor into CI against the private repo (needs an operator
  secret) — build the tool + a local invocation; CI wiring is a follow-up note.
- Any production contact.

## Git workflow

- Branch: `advisor/024-hardening`
- Commit per piece: property tests; chaos harness; extractor.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Property tests for the parser and framing

Add `proptest` as a dev-dependency to `ruxel-core` and `ruxel-proto`. Write:
- `parse_never_panics`: `proptest` generating structurally-plausible-but-arbitrary
  YAML (and raw strings) — `playbook::parse` must return either a typed model or
  a clean error, **never panic** and never accept a value outside the closed
  surface. This protects the "unknown = hard error" invariant.
- `read_frame_never_panics`: `proptest` over arbitrary byte buffers — `read_frame`
  returns `Ok(Some)`/`Ok(None)`/`Err`, never panics, never allocates beyond
  `MAX_FRAME_LEN` (the oversize guard). Complements plan 017's targeted edge
  tests.

**Verify**: `cargo nextest run -p ruxel-core` and `-p ruxel-proto` → the property
tests pass (or, if one **finds a panic**, STOP and report it — that's a real bug).

### Step 2: Design + prototype the chaos harness

Design a harness that, against a **disposable fixture** (operator-provided;
record `Safety check: target`), injects a disconnect at each protocol state
(after Hello, mid-Plan, after a TaskStart, mid-TaskResult, etc.) and asserts:
- the agent's flock is released (a subsequent run acquires it — no wedged lock),
- a rerun **converges** (idempotent — the half-applied task is re-applied
  cleanly), and
- no protocol state leaves the target unrecoverable (the M5 gate).
Prototype it using plan 017's protocol-test scaffolding (drop stdin at each
state) for the states testable **locally** (over pipes, no VM); document which
states need a real fixture (network drop) for the operator to run. This is a
**spike**: deliver a working harness for the local-testable states + a documented
plan for the fixture-only states, not necessarily full coverage.

**Verify**: the local chaos tests (drop-at-state over pipes) pass and prove the
flock releases + rerun converges for those states. Document the fixture-only
states as an operator follow-up. **If any local chaos test reveals a
non-recoverable state, STOP and report** — that's exactly the bug this gate
exists to find, and it blocks production contact.

### Step 3: Build the spec-drift extractor

Build `tools/spec-extract/` (a script — Python fits the oracle tooling, or a
small Rust bin `ruxel-spec-extract`) that, given a workload directory, walks the
16 playbooks and extracts every `(module, param, value-literal)` triple, then
diffs it against the closed registry (`crates/ruxel-core/src/modules.rs`
`MODULES` — its `params` and `literal_enums`). Output: any module/param/value in
the workload **not** covered by the registry (which is exactly what should fail
CI). Provide a local invocation and document the CI-wiring follow-up (it needs
the private `ansible-configs` checkout as a CI secret — note it; do **not** wire
it against the private repo here).

**Verify**: run the extractor against a **small synthetic** workload dir you
create in the scratchpad (a couple of playbooks using known + one unknown
param) and confirm it flags the unknown. Do **not** point it at the real private
workload unless the operator provides the path; if `RUXEL_WORKLOAD_DIR` is set
and available, a dry run against it should report **no** drift (the registry is
current) — if it reports drift, that's a real finding (report it).

### Step 4: Update the docs and full gates

Update `PLAN.md`/README (coordinate with plan 002/003): the spec-drift watch is
now **built** (local; CI-wiring is the remaining follow-up). Full gates:
`cargo fmt --all --check` → 0; `cargo clippy --all-targets -- -D warnings` → 0;
`cargo nextest run` → green.

## Test plan

- Property: `parse_never_panics`, `read_frame_never_panics` (`proptest`).
- Chaos: local drop-at-state tests (flock release + rerun converges); fixture-
  only states documented for the operator.
- Extractor: run against a synthetic workload dir (flags the injected unknown);
  optional dry run against the real workload reports no drift.

## Done criteria

ALL must hold:

- [x] `proptest` property tests for `playbook::parse` and `frame::read_frame` exist and pass (no panics, invariants held)
- [x] A chaos harness proves flock-release + rerun-convergence for the locally-testable protocol states; fixture-only states are documented for the operator
- [x] `tools/spec-extract/` exists, diffs workload triples against the closed registry, and flags an injected unknown in a synthetic test
- [x] Any bug the chaos/property tests found is reported as a separate finding (not silently patched)
- [x] `cargo nextest run` green; clippy/fmt clean
- [x] `plans/README.md` row for 024 updated; PLAN/README spec-drift claim updated to "built (CI-wiring pending)"

No parser, framing, closed-surface, flock, or local rerun-convergence bug was
found by this plan's property and chaos cases. The private workload check was
not run because `RUXEL_WORKLOAD_DIR` was unavailable.

## STOP conditions

Stop and report if:
- A property test finds a parser panic or a closed-surface escape — real bug;
  report it (blocks the "unknown = hard error" guarantee).
- A local chaos test finds a non-recoverable protocol state — **critical**; this
  blocks production contact. Report with the exact state and repro.
- The extractor can't run without the private workload and you have no path —
  build + test it against a synthetic dir and note the private-checkout
  dependency; do not attempt to obtain the private repo.

## Maintenance notes

- The chaos + property gate is what makes M6 production contact defensible —
  treat any bug it finds as high priority, ahead of feature work.
- Reviewer: the extractor's value is only realized once it runs in CI against the
  live workload (needs an operator secret) — the follow-up note must be explicit
  so it isn't forgotten.
- The chaos harness reuses plan 017's protocol scaffolding; keep them aligned.

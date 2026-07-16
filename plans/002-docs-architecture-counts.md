# Plan 002: Reconcile normative docs with the built code

> **Executor instructions**: Follow step by step; verify each step. Honor
> STOP conditions. Update this plan's row in `plans/README.md` when done.
> This plan edits documentation only — no `crates/**` changes.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- docs/ README.md`
> If any listed doc changed since `b5f98ba`, re-verify the "Current state"
> excerpts against the live text before editing; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none (independent of 001, but 001 covers AGENTS/README status)
- **Category**: docs
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

The design docs are read *as contracts* by the AI agents that build this repo.
Several normative docs describe the system in the **present tense for
mechanisms that were never built**, list **contradictory module/template
counts**, and carry **⚠-verify markers on subtleties that were already
resolved** — while `docs/PLAN.md` ties "a milestone is not done while a ⚠ item
is open." The concrete failures: an agent reads `ARCHITECTURE.md §5` and builds
on top of "batched system caches" that don't exist; reads `PLAN.md` and
re-runs parity experiments already closed; can't answer "is this module fully
covered?" because `WORKLOAD.md` says 29, its own table has 33 rows, and the
code registry asserts 36. Fixing the docs removes wasted work and false "done"
signals. **This plan never changes behavior — it makes the docs describe what
the code actually does**, marking unbuilt mechanisms as design targets.

## Current state

Each item below is a doc claim vs. the code reality (both cited so you can
confirm). Read the cited code lines first; they are the source of truth.

**A. `docs/ARCHITECTURE.md` present-tense claims for unbuilt/replaced mechanisms:**

1. §2 line 63: *"Implementation: the `openssh` crate over the system OpenSSH
   with ControlMaster native-mux."* — Reality: transport is hand-rolled via
   `tokio::process` (`crates/ruxel/src/transport.rs:1-14,54-75`); the `openssh`
   crate was dropped (`transport.rs:9` comment "the openssh crate this
   replaced"). `openssh-sftp-client` *is* still used for blob upload
   (`transport.rs` SFTP path), so word it precisely.
2. §5 lines 157-164: the "batched system caches" table (one dpkg snapshot, one
   D-Bus `ListUnits` batch, one PG connection serving all checks). — Reality:
   no cache layer exists; the agent forks a subprocess per package/unit/SQL
   statement (`crates/ruxel-agent/src/modules/apt.rs`, `systemd.rs`,
   `postgresql.rs`; and `ledger.rs` re-forks on verify). Unbuilt.
3. §2 line 88 + §6: a streamed `ProbeResult{verdict: CachedOk|NeedsCheck|
   NeedsApply}` event, and `HelloAck.ledger_gen`. — Reality: `ProbeResult` is
   **not in the proto** (`crates/ruxel-proto/proto/ruxel.proto` `Event` oneof
   has no such variant); `ledger_generation` is hardcoded `0`
   (`crates/ruxel-agent/src/main.rs:138`). Unbuilt.
4. §4 lines 132-146: register-dependency pipelining / issue windows /
   `PlanPatch` continuations. — Reality: the scheduler is linear and sends one
   task per `Plan`, blocking per task (`crates/ruxel/src/scheduler.rs:1-8` says
   so; the compiler DAG is used only by `plan`, not `apply`). `PlanPatch` is
   only *received* by the agent, never sent by the controller. Unbuilt (this
   is plan 020's work).
5. §6 lines 179-180: ledger is "append-compacted redb (or equivalent
   single-writer)." — Reality: a single `ledger.json`
   (`crates/ruxel-agent/src/ledger.rs:78,90-103`). The "(or equivalent)" hedge
   *partially* covers this; note it as JSON, don't call it a bug.
6. §7 lines 233-243: `--detailed-exitcode` flag and a per-run JSON log at
   `~/.local/state/ruxel/runs/<ts>-<run_id>.jsonl`. — Reality: neither exists
   (grep `crates/` for `detailed_exitcode` / `state/ruxel` → nothing). These
   are plan 023's work; here just mark them unbuilt.

**B. Count contradictions (module registry is authoritative:
`crates/ruxel-core/src/modules.rs` — its test asserts `MODULES.len() == 36`):**

- `docs/WORKLOAD.md:12` "## 1. Module inventory (29 distinct modules)";
  `docs/VISION.md:8` and `docs/DIRECTION.md` also say "29". `GOAL.md`/`RESTORE.md`
  and the code say **36**. WORKLOAD's own two tables total 33 rows (24 built-in
  + 9 collection) and omit `apt_repository`, `fail`, `set_fact`.
- Per-module use-count mismatches WORKLOAD ↔ SEMANTICS: `WORKLOAD.md:25`
  `group | 39` vs `docs/SEMANTICS.md:285` `group (3)`; `WORKLOAD.md:32`
  `user | 6` vs `SEMANTICS.md:280` `user (5)`; `WORKLOAD.md:31` `replace | 6`
  vs `SEMANTICS.md:206` `replace (3)`. (WORKLOAD's `group | 39` is almost
  certainly a typo — 39 is the `group` module's *table-row position artifact*,
  not a use count.)
- Template counts: `docs/PLAN.md:76,80` and `SEMANTICS.md:198` say "22
  templates"; the actual gate evidence is "41 template files (22 with Jinja)"
  (`GOAL.md:241`, `RESTORE.md:61`). The `template` module row in the WORKLOAD
  built-in table (`WORKLOAD.md:33`, `template | 41`) is also out of
  descending-use sort order.

**C. Stale ⚠-verify markers in `docs/SEMANTICS.md`** (governance:
`docs/PLAN.md:15-18` "a milestone is not done while one of its ⚠ items is
unresolved"). These carry ⚠ but are recorded closed in `GOAL.md`:
  - skip register shape `SEMANTICS.md:92` → closed `GOAL.md:249`
  - lineinfile idempotence `:205` → `GOAL.md:338`
  - systemd `daemon_reload` changed `:245` → `GOAL.md:315`
  - sysctl normalization `:253` → `GOAL.md:338`
  - authorized_key key-material `:288` → `GOAL.md:341`
  - get_url dest short-circuit `:217` → resolved inline at `:217` itself
  - lvg `:258`, lvol `+100%FREE` `:266`, mount fstab `:275` → `GOAL.md:353-354`

  **Genuinely still open (keep ⚠):** pause under `--check` `:160`, handlers
  under `--check` `:164` (live `--check` is unbuilt), `become_user` env `:32`,
  file `mode "0700"` octal `:192` (not pinned in GOAL/RESTORE).

**D. Broken claims in `docs/PLAN.md` / `README.md`:**
- `PLAN.md:158-162` "Spec drift watch: the param/value extractor … lives in
  `tools/spec-extract/` and runs in CI." — `tools/spec-extract/` does not exist
  (`tools/` has only `fixtures/` and `oracle/`); no CI job references it.
  `README.md` item 7 advertises "the spec-drift CI watch." (Building the tool
  is plan 024/025 territory; here, mark it *planned, not built*.)
- `README.md:20-39` lists 7 docs but omits `docs/OPERATOR-SETUP.md` and
  `docs/benchmarks/`; `README.md:38` calls the plan "M1–M6" but `PLAN.md:22`
  defines **M0** as the first milestone.
- `docs/OPERATOR-SETUP.md:49` says fixtures are "CX-line"; the scripts default
  to `cpx12` (`tools/fixtures/lib.sh:15`, comment "no cx-line in this
  account").

**Convention**: match each doc's existing tone. Prefer a small, clearly
labeled "Current status vs. design target" note over rewriting whole sections,
so the *design intent* is preserved while the *build state* is honest.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Confirm registry count | `grep -n "len(), 36\|== 36" crates/ruxel-core/src/modules.rs` | matches (36 is authoritative) |
| Confirm no spec-extract | `ls tools/` | only `fixtures  oracle` |
| Confirm openssh dropped | `grep -rn "use openssh" crates/ruxel/src` | no matches |
| Confirm no ProbeResult | `grep -rn "ProbeResult" crates/` | no matches |
| Review edits | `git diff docs/ README.md` | only intended text changes |

## Scope

**In scope**: `docs/ARCHITECTURE.md`, `docs/SEMANTICS.md`, `docs/WORKLOAD.md`,
`docs/VISION.md`, `docs/DIRECTION.md`, `docs/PLAN.md`, `docs/OPERATOR-SETUP.md`,
`README.md` (doc-list + milestone-range only; the Status paragraph is plan
001's).

**Out of scope**:
- Any `crates/**` file, `Cargo.toml`, CI config — docs only.
- Building `tools/spec-extract/` — that's plan 024/025; here only correct the
  claim that it already exists.
- `GOAL.md`, `RESTORE.md` — they are the *fresher* truth; do not "reconcile"
  them down to the stale docs. If they disagree with a design doc, the design
  doc is the one to annotate.

## Git workflow

- Branch: `advisor/002-docs-reconcile`
- Commit in logical slices (e.g. one for ARCHITECTURE, one for counts, one for
  ⚠ markers) or one squashed `docs: reconcile design docs with built code`.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Add "current vs designed" annotations to ARCHITECTURE.md

For each of A.1–A.6, insert a short bracketed status note where the mechanism
is described — do **not** delete the design prose (it is the intended future
shape). Example for §5:

```markdown
> **Build status (2026-07): NOT YET BUILT.** The batched caches below are the
> design target. Today each module forks a subprocess per package/unit/SQL
> statement; the shared snapshots are plan 021. See plans/021.
```

For §2 openssh: correct the sentence to name `tokio::process` as the connection
driver and `openssh-sftp-client` for blobs; note the `openssh` crate was
dropped. For §6 redb: change "redb (or equivalent single-writer)" to note the
current implementation is `ledger.json` (single-writer via flock).

**Verify**: `grep -n "NOT YET BUILT\|tokio::process\|ledger.json" docs/ARCHITECTURE.md`
→ annotations present; `git diff docs/ARCHITECTURE.md` shows only added notes
and the §2 correction, no deleted design paragraphs.

### Step 2: Pick one canonical module count and fix WORKLOAD/VISION/DIRECTION

State **36** as the canonical closed-surface module count everywhere, with a
one-line note on what it counts (the 33 table rows + `apt_repository`, `fail`,
`set_fact`, counting `sysctl`/`ansible.posix.sysctl` as two spellings). Fix the
`group | 39` → `group | 3`, `user | 6` → `user | 5`, `replace | 6` →
`replace | 3` rows in `WORKLOAD.md` to match `SEMANTICS.md §6`. Re-sort the
`template | 41` row into descending-use order (or add a footnote that the
built-in table mixes uses and file-counts). Split "41 template files (22 with
Jinja)" consistently in `PLAN.md:76,80` and `SEMANTICS.md:198`.

**Verify**: `grep -rn "29 distinct modules\|29 modules" docs/` → no matches;
`grep -n "group | 39" docs/WORKLOAD.md` → no matches.

### Step 3: Clear the resolved ⚠ markers in SEMANTICS.md

For each of the nine closed items in section C above, remove the `⚠ verify`
glyph/label and append the reference that closed it (e.g.
`(pinned: GOAL.md session 3, runtime goldens)`). **Leave the four genuinely
open ones** (pause/handlers under `--check`, become_user env, file mode octal)
marked ⚠ and add an explicit "Still open" list at the end of §6 so they are not
lost. Verify each against `GOAL.md` before clearing — do not blanket-delete.

**Verify**: `grep -c "⚠" docs/SEMANTICS.md` → count dropped from the original
(should equal the 4 still-open items plus any ⚠ used in prose headings you did
not touch); the four open items are still present.

### Step 4: Soften the spec-drift claim and fix the README doc list

In `PLAN.md:158-162` and the `README.md` item 7, change the present-tense
"lives in `tools/spec-extract/` and runs in CI" to "**planned** (not yet
built — see plans/024)". Add `docs/OPERATOR-SETUP.md` and `docs/benchmarks/`
to the README doc-reading list; change "milestones M1–M6" to "M0–M6". Fix
`OPERATOR-SETUP.md:49` "CX-line" → "cpx12 (smallest available; no CX-line in
this account), sin region".

**Verify**: `ls tools/` still shows no `spec-extract` (you did not create it);
`grep -n "planned\|not yet built" docs/PLAN.md` → the spec-drift note is now
conditional.

## Test plan

No code; verification is by grep + `git diff` review (commands above). There is
no automated doc-lint in this repo, so correctness is by inspection against the
cited code lines.

## Done criteria

ALL must hold:

- [x] `grep -rn "29 distinct modules\|29 modules" docs/` → no matches
- [x] `grep -n "group | 39" docs/WORKLOAD.md` → no matches
- [x] ARCHITECTURE §2/§4/§5/§6/§7 each carry a build-status note for the unbuilt/replaced mechanism
- [x] The 9 resolved ⚠ items are cleared; the 4 open ones remain and are listed
- [x] `PLAN.md`/README spec-drift claim reads as "planned", not "runs in CI"
- [x] No `crates/**`, `Cargo.toml`, or `.github/**` file modified (`git status`)
- [x] `plans/README.md` row for 002 updated

## STOP conditions

Stop and report if:
- A cited code line no longer matches (e.g. `ProbeResult` now exists, or the
  openssh crate is back in `transport.rs`) — the code changed since `b5f98ba`;
  the doc reconciliation direction may be wrong.
- `GOAL.md` and a design doc disagree on a ⚠ closure and you cannot determine
  which is right from the cited golden — flag it rather than guessing which to
  trust.
- You find the registry no longer asserts 36 modules — recount before writing
  a canonical number.

## Maintenance notes

- The durable fix for count drift is the spec-drift extractor (plan 024): once
  it runs in CI, these counts stop needing manual reconciliation. This plan is
  the stopgap.
- Reviewer: verify no design *intent* was deleted — every "NOT YET BUILT" note
  should sit *beside* the preserved design prose, not replace it. The docs must
  still describe the target architecture (plans 020/021/023 implement it).

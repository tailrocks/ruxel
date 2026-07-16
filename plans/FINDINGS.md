# Ruxel — Deep Audit Findings Inventory

Complete inventory of everything the deep audit surfaced, against commit
`b5f98ba` (2026-07-03). Eight parallel read-only auditors (correctness,
security, performance, tests, tech-debt, deps/DX, docs, direction) swept the
workspace; every finding here was re-read and confirmed by the advisor before
inclusion. This document is the **reference map**; the actionable specs are the
numbered plans in this directory (`plans/NNN-*.md`), cross-referenced per
finding.

**Audited:** all 4 crates (~7.3k LOC Rust), `crates/ruxel-proto/proto/ruxel.proto`,
`tools/fixtures/*.sh`, `tools/oracle` own scripts + committed `captures/*.jsonl`,
`.github/workflows/ci.yml`, all root/`docs/` markdown.
**Not audited:** vendored `tools/oracle/{.venv,galaxy,collections}`; the private
`ChainArgos/ansible-configs` workload itself (not in-repo — several findings'
*reachability* is therefore MED, marked inline); live runtime behavior (read-only
audit — no VM, no production contact, per `AGENTS.md`).

**Confidence legend:** HIGH = read the code, certain · MED = strong signal,
workload-incidence unconfirmed · LOW = smell, investigate.

---

## Cross-cutting theme

The **convergence ledger is both the headline feature ("plan in seconds") and
the most dangerous, least-tested subsystem.** Four confirmed silent-drift bugs
let a *drifted production host* report `ok (ledger)` and never re-converge — the
opposite of the tool's promise — and it has **zero tests**. Two intent docs
(`GOAL.md`, `RESTORE.md`) also **oversold** two capabilities the code does not
deliver: "secret lookups memoized to a handful of `op` calls" (serial + lazy —
[F-PERF-02]) and "all four postgresql_privs shapes idempotent, rerun changed=0"
(`default_privs` hardcoded always-changed — [F-CORR-03]). For those two claims,
the **code is ground truth**, not the docs.

---

## Tier 1 — ledger silent-drift + secret-at-rest (P1)

| ID | Finding | Sev | Conf | Evidence | Plan |
|----|---------|-----|------|----------|------|
| F-CORR-01 | Ledger `File` probe records only content hash+len, ignores `mode`/`owner`/`group` → converged file whose perms drift re-verifies content only → cache hit, declared mode never restored (also: `state=directory` disables caching entirely — `std::fs::read` on a dir fails) | HIGH | HIGH | `ledger.rs:21-25,44-48,160-172`; module enforces attrs at `modules/mod.rs:242-274` but `main.rs:232-239` replays cached w/o invoking it | 005, 006 |
| F-CORR-02 | Ledger caches `apt state=latest` (no `state` check in probe) → once converged, newer upstream package never detected; `latest` silently degrades to `present` (violates ARCHITECTURE §6 network-truth honesty rule) | HIGH | HIGH | `ledger.rs:173-186` | 005, 006 |
| F-SEC-02 / F-CORR-07 | Agent never reads `no_log`; records every non-check task's full result JSON into the ledger, and for `copy`/`template` under `--diff` that result carries **full plaintext** file content → secret written at rest | HIGH | HIGH | `main.rs:277-285` (unconditional record); `copy.rs:31-35` (plaintext diff); proto `no_log` field 7 set by controller (`scheduler.rs:598`) but unread agent-side | 008 |
| F-SEC-03 | `ledger.json` written world-readable (no `set_permissions`, default 0644); dir 0755 → secret-bearing results readable by any local user on production hosts | HIGH | HIGH | `ledger.rs:90-103`; `main.rs:56-57` | 008 |
| F-SEC-04 | Ledger key is an **unkeyed** `blake3` of rendered params (incl. secret-bearing file content) — §6's `HMAC(host_ledger_key, value)` substitution never implemented → weak embedded secret offline-guessable from the ledger | MED | HIGH | `scheduler.rs:566-581`; spec at `ARCHITECTURE.md:183-186` | 008 |
| F-CORR-10 | Ledger `sysctl` probe verifies only the **file** value, not the live `/proc/sys` value the module enforces under `sysctl_set: true` → live-value drift survives cache | MED | MED | `ledger.rs:199-208` vs module `sysctl.rs:66-84` | 006 |
| F-CORR-06 / F-DEBT-10 | Ledger flushed **only** on `Done`; a controller Ctrl-C / link drop → clean EOF → agent returns 0 **without flush** → all in-run fingerprints lost → next run re-checks everything (defeats the headline speed on any interrupt). Contradicts ARCHITECTURE §8 "writes the ledger, exits" | HIGH | HIGH | `main.rs:104-115` (`Ok(None)=>return 0`) vs flush at `:145-147` | 007 |
| F-TEST-01 | The entire ledger (`ledger.rs`, 281 lines) has **zero tests** — no coverage of cached_ok / verify / probe_for / corrupt-recovery / version-invalidation | — | HIGH | grep: no `#[test]` in `ledger.rs` | 005 |
| F-CORR-15 / F-TEST-05 | `task_eval::censored_result` (no_log redaction helper) is tested (goldens E12/E13) but has **no production caller** — the tested path is dead code; production redaction is ad-hoc in the scheduler | LOW | HIGH | `task_eval.rs:123-141` + grep (no callers) | 008 |

---

## Tier 2 — scheduler control-flow + architecture (P1/P2)

| ID | Finding | Sev | Conf | Evidence | Plan |
|----|---------|-----|------|----------|------|
| F-CORR-04 | A `register`ed single-shot task that `when`-skips, and a `register`ed empty-loop task, **never bind their variable** (early `return` before the register bind) → downstream `reg.skipped`/`reg is defined` errors or evaluates opposite to Ansible. Violates SEMANTICS §3.8 | HIGH | HIGH | `scheduler.rs:245-256,258-263` return before bind at `:284-286`; `finish_task` (`:634-673`) doesn't register | 009, 010 |
| F-CORR-05 | Block-level `become`/`become_user`/`vars`/`environment` **not inherited** by child tasks (only tags + when threaded) → children under a `become_user: postgres` block run as **root** (wrong ownership / psql peer-auth). Violates SEMANTICS §4 | HIGH | MED | `scheduler.rs:169-209` | 009, 010 |
| F-CORR-13b | Block `always:` is destructured `_` and **never run** — latent (workload doesn't use `always`), but parser/compiler accept it → silent cleanup-loss on any future edit | LOW | HIGH | `scheduler.rs:172` | 010 |
| F-PERF-01 / F-DEBT-01 | `apply` **never consumes the compiler** — the register-dependency DAG exists (`compiler.rs`) but is used only by `plan`; the scheduler sends one task per `Plan` and blocks one RTT per task *and per loop-item*. Also: apply-time closed-surface enum re-validation is absent (plan catches out-of-surface templated `state:`, apply doesn't). ~1.5–4 s serialized round-trip stall on a 65-task setup-* run | HIGH | HIGH | `scheduler.rs:583-629` (single `.send`); `compiler::compile` only at `plan.rs:54`; `validate_rendered_enums` at `compiler.rs:314-338` unused by apply | 009, 020 |
| F-PERF-02 | Secrets resolved by **serial per-field `op read` subprocess, lazily, every run** (incl. converged no-op) — no concurrent/grouped-per-item fetch. ~52 lookups for setup-postgresql-nova ≈ **15–30 s** serial, vs §10's 1–3 s budget. (Docs claim this is "memoized" — it is not) | HIGH | HIGH | `secrets.rs:59-74`; `engine.rs:96-140`; no batch phase in `apply.rs:83-89` | 021 |
| F-PERF-03 | No agent-side system snapshots (ARCHITECTURE §5 unbuilt) — forks `dpkg-query`/`systemctl`/`psql` per package/unit/statement; ledger re-forks on verify. Even the cached path forks ~66 subprocesses to *verify* a 65-task run | HIGH | HIGH | `apt.rs:188,202`, `systemd.rs:32,47`, `postgresql.rs:25-37`, `ledger.rs:233-264`; no cache structs in agent | 021 |
| F-PERF-04 | Probes evaluated **sequentially** in a single-threaded agent (§6's "concurrent whole-plan" absent) — gated behind F-PERF-01 (agent never has >1 task's probes at once) | MED | HIGH | `ledger.rs:113`; agent `Cargo.toml` has no tokio/rayon | 021 |
| F-PERF-06 | Hosts run **strictly sequentially** in `apply` (independent of the known transport stall) — ARCHITECTURE §1 "hosts in parallel" absent from the code shape → 6-host run is 6× single | HIGH | HIGH | `apply.rs:124-193` (`for host` awaits each) | 022 |

---

## Tier 3 — module correctness (P2)

| ID | Finding | Sev | Conf | Evidence | Plan |
|----|---------|-----|------|----------|------|
| F-CORR-03 | `postgresql_privs type=default_privs` hardcoded `=> true`, no `pg_default_acl` query exists → all 7 `default_privs` tasks report `changed` **every run**, re-issuing `ALTER DEFAULT PRIVILEGES`. Contradicts the *pinned* SEMANTICS "all four shapes idempotent" claim | HIGH | HIGH | `postgresql.rs:392`; grep: no `pg_default_acl` | 011 |
| F-CORR-08 | `postgresql_user` flag idempotence (`flags_changed`) SELECTs 4 columns but compares **only `rolsuper`** → drift in CREATEDB/CREATEROLE/REPLICATION/LOGIN never detected (silent privilege drift) | MED | MED | `postgresql.rs:304-320` | 011 |
| F-SEC-01 / F-CORR-16f | `role_attr_flags` not allowlisted (unlike `privs` → `validate_privs`) — `flags_to_sql` re-emits tokens verbatim into `CREATE/ALTER ROLE` fed to `psql -f -` (`;`-separated) → a `;`-bearing value = arbitrary SQL as the postgres superuser (escalation beyond Ansible) | HIGH mech / MED reach | HIGH | `postgresql.rs:295-302` vs allowlist template at `:60-86` | 011 |
| F-CORR-12 | `lvol` create with `size: +100%FREE` forwards the raw `+` to `lvcreate -l` (the `+` is lvextend-only) → **fresh-disk provisioning fails** (only the rerun/extend path was gated) | MED | MED | `lvol.rs:83-87` | 012 |
| F-CORR-09 | `replace` uses regex `$`/`${n}` replacement semantics (`regex_lite`) — Ansible `re.sub` treats `$` literal → `$PATH`/`$HOME` in `replace:` corrupted; `\1` backrefs emitted literally | MED | MED | `replace.rs:17` | 012 |
| F-CORR-13 | `file` with no `state` defaults to `"file"` which hits the `other =>` error arm → an attrs-only `file:` task parses fine then **hard-fails at execution** (Ansible's default applies attrs) | MED | MED | `file.rs:13,67` | 012 |
| F-CORR-11 | Playbook/inventory parse errors exit **1**, not the documented **2** (propagate as `anyhow` out of `main() -> Result`) → scripts can't distinguish "host failed" (retry) from "bad playbook" (fix). clap usage errors already exit 2 | MED | HIGH | `main.rs:25-31`; parse sites `apply.rs:71,79`, `plan.rs:40,49,54` | 013 |
| F-CORR-16 | Low-confidence investigate cluster (each read, real, incidence unconfirmed): `command` swallows unclosed quotes (`command.rs:80-88`); `ansible.posix.sysctl` registry omits `sysctl_file` though SEMANTICS lists it → a playbook using it would parse-error (`modules.rs:44-50` vs `sysctl.rs:26`); `filesystem` xfs `resizefs` is a no-op (`filesystem.rs:65-71`); `iptables` append-only, no insert/`rule_num` (`iptables.rs:56-60`); `blockinfile create:no` on a missing file errors vs Ansible-unchanged (`blockinfile.rs:19-23`); `apply --check` uses fake/dry secrets so check can't verify secret-dependent state (`apply.rs:54-64`) | LOW | LOW-MED | (as cited) | 012 |
| F-CORR-14 | Failed lazy var render returns a truthy poison object, not undefined → a play var that fails to render but is used in `when` silently evaluates true; a variable cycle renders a placeholder string into a file instead of erroring | LOW | LOW-MED | `engine.rs:222-238,405-415` (confirm before acting) | 012 |

---

## Security (non-ledger)

| ID | Finding | Sev | Conf | Evidence | Plan |
|----|---------|-----|------|----------|------|
| F-SEC-05 | `authorized_key` write+chmod+`chown` (not `lchown`) **follow symlinks** → pre-planted `~user/.ssh/authorized_keys` symlink makes root truncate + chown an attacker file to the unprivileged uid (privesc). Ansible uses atomic in-dir rename | MED | HIGH mech | `authorized_key.rs:47-50` | 014 |
| F-SEC-11 | Content modules (`lineinfile`/`blockinfile`/`replace`/`sysctl`/`mount`) write via `std::fs::write` (symlink-follow); `sysctl`/`mount` build single-line config from params where an embedded newline injects extra entries | LOW-MED | MED | `blockinfile.rs:51`, `replace.rs:21`, `sysctl.rs:62`, `mount.rs:54,26` | 014 |
| F-SEC-07 | `filesystem` builds `mkfs.{fstype}` (program name) from a rendered param with **no allowlist** (workload uses only xfs/ext4) | LOW | MED | `filesystem.rs:52-53` | 014 |
| F-SEC-06 | SSH ControlMaster mux socket in world-writable `/tmp` with a predictable `pid+nanos` name (Ansible uses a 0700 user-private dir) — multi-user-controller race/DoS against the socket carrying root SSH sessions | LOW | MED | `transport.rs:54-61` | 015 |
| F-SEC-08 | Content-addressed agent trusted by **filename**; no re-hash of the remote file before spawn (defense-in-depth gap — only root writes the dir) | LOW | MED | `transport.rs:317-324` | 015 |
| F-SEC-10 | `Debug` derived on secret-bearing `engine.rs` types (`VarValue`/`Scope`/`ScopeObject` whose memo holds resolved secrets) — latent leak if ever `dbg!`'d (no active site today) | LOW | LOW | `engine.rs:154,162,204` | 015 |
| F-SEC-09 | `tools/oracle/captures/pg-bless.jsonl` embeds a 9-char literal `looker` password (not a `dry-secret` fake); other captures embed real `op://ChainArgos/...` item refs + a GCP key filename (reconnaissance metadata) | LOW-MED | MED | (value never reproduced) | 015 |

**F-SEC-09 — operator-resolved (2026-07-03):** the operator confirmed the
`pg-bless` capture was made against a disposable **test server** (`91.99.37.240`)
used to verify the workflow, not a production host → the `looker` value is a
test-server credential, **not** a production secret. **No rotation required.**
Downgraded to hygiene: capture with `RUXEL_DRY_SECRETS=1` going forward. The
committed `op://` refs / GCP key filename remain a minor "is this intended?"
question for the operator.

**F-SEC-06/08/10 — resolved (2026-07-16):** ControlMaster sockets now live in
a mode-0700 XDG runtime or home directory; cached and freshly uploaded agents
are SHA-256 verified remotely before execution; secret-bearing engine scope
types use redacted manual `Debug` implementations with regression coverage.

**Security leads that checked out CLEAN** (verified complete, not findings):
git argv flag-smuggling guards (commit 670ece4); get_url `--` + scheme check;
apt_repository filename validation; PG SQL-via-stdin (no password in argv) +
privs allowlist + identifier quoting; `command` argv-exec (no shell); `secrets.rs`
returns values via stdout not argv, logs identity-only; recap/JSON print only
status counts; transport BatchMode fails-closed on host-key mismatch, SFTP
`O_EXCL` temp+rename; CI actions SHA-pinned, `permissions: contents: read`, no
`OP_SERVICE_ACCOUNT_TOKEN` in workflows; fixtures ssh-keygen 0600, no `set -x`
leak, no `curl|bash`; no prompt-injection content in captures/docs/fixtures.

---

## Tests, docs, deps/DX, tech-debt

| ID | Finding | Sev | Conf | Evidence | Plan |
|----|---------|-----|------|----------|------|
| F-TEST-02 | `scheduler.rs` (758 lines, the control-flow heart) has **zero tests**; only on-VM bless-gate over 6/16 playbooks. Reading it surfaced F-CORR-04 and F-CORR-13b | — | HIGH | grep; `ruxel-cli` lib = 0 tests | 009 |
| F-TEST-03 | 18 of 24 agent module files have **zero** unit tests; the 6 "tested" cover one pure helper each. Riskiest untested: PG ACL/flags, `user` group reconcile, storage modules | — | HIGH | coverage matrix | 016 |
| F-TEST-04 | The agent's task-execution + ledger path is never exercised off-VM — `protocol.rs` sends only `Hello`+`Done`, never a `Plan` | — | HIGH | `protocol.rs` | 007, 017 |
| F-TEST-07 | CI's `nextest ... --no-tests=pass` is green while **silently skipping** the workload compile gate + 41-template parity gate (they early-`return` as "passed" without `RUXEL_WORKLOAD_DIR`); `--no-tests=pass` masks a test-empty crate | — | HIGH | `workload.rs:14-17,80-82`, `render_parity.rs:168-171`, `ci.yml:65` | 004 |
| F-TEST-08 | `frame.rs` edge branches untested: mid-varint EOF, varint overflow, `Interrupted` retry | LOW | HIGH | `frame.rs:31-52` | 017 |
| F-DOCS-01 | `AGENTS.md` (auto-loaded via `CLAUDE.md` every session) still says "Research and design. Do not start building the execution engine" — an agent obeying it **refuses to code** the already-built engine | HIGH | HIGH | `AGENTS.md:20-25` vs `GOAL.md:112` | 001 |
| F-DOCS-02 | README Status: "the engine intentionally does not yet [exist]" — stale by a full implementation | — | HIGH | `README.md:16-19` | 001 |
| F-DOCS-04/05 | ARCHITECTURE describes unbuilt/replaced mechanisms in present tense (openssh crate [dropped], §5 batched caches, `ProbeResult` event, §4 pipelining, redb ledger, §7 run log + `--detailed-exitcode`) | — | HIGH | (as cited in plan 002) | 002 |
| F-DOCS-06/07 | Contradictory counts (29 vs 33-rows vs 36 modules; `group 39` vs `3`; 22 vs 41 templates); stale ⚠-verify markers on 9 already-closed items (governance ties "milestone done" to ⚠ closure → agents re-run closed experiments) | — | HIGH | WORKLOAD/SEMANTICS/PLAN vs `modules.rs` | 002 |
| F-DOCS-03/08/09 | `tools/spec-extract/` claimed to "run in CI" but absent; README doc-list omits OPERATOR-SETUP + benchmarks, mislabels "M1–M6" (M0 is first); OPERATOR-SETUP says "CX-line" but scripts default cpx12 | LOW | HIGH | `PLAN.md:158`, `README.md:20-39`, `OPERATOR-SETUP.md:49` | 002, 024 |
| F-DEPS-01 / F-DEBT-04 | Dead deps: `openssh` (+native-mux subtree, replaced by tokio::process) and `anyhow` (agent) declared but unused | — | HIGH | `Cargo.toml:27`; grep | 003 |
| F-DEBT-09 | Dead code: `EngineError::{VarCycle,Lookup}` never constructed; `transport::connect` no callers; `HostFacts.{agent_version,ledger_generation}` written-never-read | — | HIGH | `engine.rs:22,26`, `transport.rs:191,186-187` | 003 |
| F-DEBT-03 | Dead protocol surface: `BlobsNeeded`/`PauseRequest` never sent, `PlanPatch`/`Resume` only received (unbuilt §4/§6 mechanisms) | — | HIGH | proto vs usage | 003, 020 |
| F-DEPS-04 | No `cargo-deny`/`cargo-audit` CI gate (Apache-2.0 + GPL clean-room rule, large transitive subtrees) — no advisory or license-contamination detection | — | HIGH | `ci.yml` (4 jobs) | 004 |
| F-PERF-05 / F-DX-02 | `clippy` + `agent-cross` CI jobs cold-build the whole dep graph (no target cache/sccache); target cache key includes `head_ref` (branch churn); redundant `cargo build` after nextest | — | HIGH | `ci.yml:87-147,55-67` | 004 |
| F-DEBT-02 | ~14–20 hand-rolled `Command::new().output()`+stderr-capture copies across agent modules (no shared runner) | — | HIGH | grep counts | 018 |
| F-DEBT-07 | Varint frame codec implemented **twice** (sync `frame.rs`, async `transport.rs`) and **already drifted** (`Interrupted` handling) | MED | HIGH | `frame.rs:14-65` vs `transport.rs:379-419` | 018 |
| F-DEBT-05 | Three hand-synced module registries (core `MODULES`, agent dispatch, ledger `probe_for`) with no cross-check → a missing agent arm fails only at runtime on a live host | — | HIGH | `modules.rs` / `modules/mod.rs:96-130` / `ledger.rs:157-211` | 018 |
| F-DEBT-08 | Coupled magic literals apart: `/var/lib/ruxel` in 2 crates; `/etc/sysctl.conf` defaulted independently in `sysctl.rs` (writer) and `ledger.rs` (probe reader) — divergence silently breaks cache verify; two 30s handshake timeouts apart | LOW | HIGH | `transport.rs:25`, `main.rs:45`, `sysctl.rs:26`, `ledger.rs:201` | 018 |
| F-DX-01 | `RUXEL_WORKLOAD_DIR` undocumented in README → the parity gates silently skip (green-but-not-run) | LOW | MED | `README.md:56-66` vs `workload.rs:14` | 004 |
| F-DX-03 | No justfile/pre-commit/editorconfig/`cargo-machete` — quality gates are manual prose (a past clippy slip-through recorded) | LOW | MED | absent | 019 |
| F-DX-07 | No release/versioning/tagging story — no git tags, no CHANGELOG, all `0.1.0`; "which build is deployed?" unanswerable | LOW | HIGH | `git tag -l` empty | 019 |
| F-DX-08 | Oracle Python interpreter unpinned (venv 3.13 vs lock cp312; `uv=latest`) — hermeticity gap for a parity oracle | LOW | LOW | `pyproject.toml:5`, `.venv` | 019 |
| F-DEPS-03 | Renovate doesn't cover the `rust-toolchain.toml` channel pin (silently goes stale) | LOW | MED | `renovate.json` | 019 |

**Clean leads noted:** `engine.rs`/`scheduler.rs`/`postgresql.rs` are large but
cohesive (not god objects); error handling is consistent by layer (anyhow at
CLI, thiserror in core, `Result<_,String>` in agent); fixture bash is hardened
(`set -euo pipefail` via `lib.sh`); no commented-out code; `regex-lite` (not full
`regex`) in the agent runtime; `.gitignore` hygiene correct (`.venv`/`galaxy`
untracked).

---

## Direction findings (options for the maintainer, not defects)

| ID | Direction | Grounding | Plan |
|----|-----------|-----------|------|
| F-DIR-01 | Chaos + fuzz/property hardening **before** any production contact (the M5 "no protocol state leaves a target unrecoverable" gate; §8 re-entrancy asserted but untested) | `PLAN.md:140-145`, absent in tree | 024 |
| F-DIR-02 | Run log (`~/.local/state/ruxel/runs/*.jsonl`, redacted) + `--detailed-exitcode` — promised (§7), absent, cheap (reuse `--output json`); the M6-pilot forensic substrate | `ARCHITECTURE.md:233-243`, grep-absent | 023 |
| F-DIR-03 | Fix the multi-host transport stall → unblocks the 6-host M5 benchmark + fleet `apply` (4 playbooks target `all`). Honest: M6 pilot is per-host and `--limit` works today, so M5-proof + ergonomics, not a pilot blocker | `transport.rs:8-15`, `apply.rs:124`, `PLAN.md:138` | 022 |
| F-DIR-04 | `--diff` for lineinfile/replace/blockinfile (23 sites incl. fstab/pam/ssh) — the pilot's core diff-review value; helper already exists | `RESTORE.md:115`, `copy.rs:31-34` vs the line modules | 023 |
| F-DIR-05 | Sweep the available-now parity gates (5 init-*-drives + restart-blockchain) — zero operator dependency, harness exists, moves M4 toward 16/16 gated | `RESTORE.md:113-114`, `WORKLOAD.md:137-148` | 025 |
| F-DIR-06 | Build the promised-but-absent spec-drift watch (`tools/spec-extract/`) — the closed spec's *enforcement* mechanism | `PLAN.md:158-162`, absent | 024 |

**Direction rejected** (recorded so nobody re-audits): "support more Ansible"
(anti-goal — closed spec); cross-task batching of shell/command (violates
ARCHITECTURE §6 honesty rule); warm-daemon tier now (deferred by design until
M5); autonomous setup-* gate sweep (blocked on operator deploy key — see below).

---

## Operator actions (not autonomous)

1. **`pg-bless.jsonl` credential — RESOLVED (2026-07-03):** operator confirmed
   the capture is from a disposable test server (`91.99.37.240`), not
   production. No rotation required. Hygiene only: capture future PG bless
   goldens with `RUXEL_DRY_SECRETS=1`.
2. **setup-* gate sweep — BLOCKED:** needs a read-only ChainArgos deploy key in
   the `ruxel-test` 1Password vault (the setup-* playbooks `git clone` private
   repos; `--dry-secrets` supplies a fake key that can't authenticate — Ansible
   fails identically). Plan 025 covers only the gates that need no key.

---

## How to use this document

- **This file** is the searchable inventory (what + where + severity).
- **`plans/NNN-*.md`** are the executable specs (how to fix, step by step, for a
  zero-context executor), cross-referenced in the tables above.
- **`plans/README.md`** is the execution order + dependency graph + status table.
- Recommended start: the P1 correctness+safety spine — 005→006/007/008 (ledger),
  009→010 (scheduler), 011 (postgresql).

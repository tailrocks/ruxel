# Plan 011: Fix PostgreSQL idempotence (default_privs, role flags) + flag allowlist

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. This plan
> has a **security** step (Step 3) — be precise; never put a real credential in
> a test.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- crates/ruxel-agent/src/modules/postgresql.rs`
> If changed, re-verify excerpts; on mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW (narrows always-changed / always-only-SUPERUSER branches; adds a
  reject for malformed flags)
- **Depends on**: none (independent; but read the ledger honesty rule — PG tasks
  are not cached, so no ledger interaction)
- **Category**: bug (correctness) + security
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Three issues in `postgresql.rs`, one of which contradicts a *pinned* SEMANTICS
claim:

1. **CORRECTNESS-03 (default_privs never idempotent)**: the `privs` dispatch
   hardcodes `"default_privs" => true`, and **no `pg_default_acl` query exists**
   — so all 7 `default_privs` tasks report `changed` on **every** run and
   re-issue `ALTER DEFAULT PRIVILEGES`. This directly contradicts
   `SEMANTICS.md §6`'s pinned guarantee ("all four shapes … ruxel rerun
   changed=0") and `GOAL.md`/`RESTORE.md`. It breaks converged-no-op reporting
   and spuriously fires any `notify` on every PG host.
2. **CORRECTNESS-08 (role flag idempotence checks only SUPERUSER)**:
   `flags_changed` SELECTs four role columns but compares **only `rolsuper`**,
   with a comment "The workload only toggles SUPERUSER; broaden if more flags
   appear." Drift in `CREATEDB`/`CREATEROLE`/`REPLICATION`/`LOGIN` is never
   detected → silent privilege drift.
3. **SECURITY-01 (role_attr_flags not allowlisted)**: `flags_to_sql` splits on
   `,`/space and re-emits the tokens **verbatim** into `CREATE ROLE`/`ALTER
   ROLE`, which is fed to `psql -f -` (executes `;`-separated statements). A
   `role_attr_flags` value carrying a `;` survives (not a split delimiter) and
   becomes arbitrary SQL as the postgres superuser — an escalation beyond what
   Ansible allows (Ansible allowlists role flags). The sibling `privs` path is
   already allowlisted (`validate_privs`); `role_attr_flags` is the gap.

## Current state

`crates/ruxel-agent/src/modules/postgresql.rs`:
- SQL runs via `psql(ctx, port, db, sql)` (`:25-55`) feeding the statement on
  **stdin** (`-f -`) — good, no secret in argv. `psql -tA` returns tuples-only.
- `validate_privs` (`:60-86`) — the allowlist template: uppercases each
  comma-split token, rejects anything not in `ALLOWED`. Called at `:375`.
- `lit`/`ident` (`:89-96`) — SQL literal/identifier quoting.
- `flags_to_sql` (`:295-302`):
  ```rust
  fn flags_to_sql(flags: &str) -> String {
      flags.split([',', ' ']).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
  }
  ```
  Called unquoted into `CREATE ROLE ... {flags_to_sql}` (`:187-189`) and
  `ALTER ROLE ... {flags_to_sql}` (`:204`).
- `flags_changed` (`:304-320`): SELECTs `rolsuper,rolcreaterole,rolcreatedb,
  rolreplication`, compares only `cols.first()` (`rolsuper`).
- `privs` dispatch (`:378-398`):
  ```rust
  let needed = match typ {
      "database" => privs_missing_database(...)?,
      "schema"   => privs_missing_schema(...)?,
      "table"    => privs_missing_table(...)?,
      "default_privs" => true, // pg_default_acl compare below decides; see grant
      other => return Err(...),
  };
  ```
  There is **no** "compare below" — `needed` is used directly at `:401`.
- `privs_missing_database`/`_schema`/`_table` (`:419+`) implement the
  aclexplode-based comparison (the comment at `:412-417` explains: idempotence
  on the *explicit* ACL grant to the role via `aclexplode(datacl/nspacl/relacl)`
  filtered to the role oid, excluding PUBLIC/inherited). **Mirror this pattern**
  for default privileges using `pg_default_acl.defaclacl`.
- `grant_sql(typ, role, privs, objs, schema)` (`:404`) builds the GRANT/ALTER
  DEFAULT PRIVILEGES statements executed when `needed`.

**Convention**: idempotence is decided in SQL against pg_catalog; "changed"
must reflect a real catalog delta. Identifiers via `ident`, literals via `lit`.
Fixtures for PG are on-VM (port 40000) — the unit-testable parts here are the
**pure** functions (`flags_to_sql`/allowlist, the SQL string a comparison
builds); the live catalog behavior is proven by the on-VM bless-gate.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| PG module tests | `cargo nextest run -p ruxel-agent postgres` | pass |
| Build | `cargo build -p ruxel-agent` | exit 0 |
| Clippy/fmt | `cargo clippy -p ruxel-agent --all-targets -- -D warnings` / `cargo fmt --all --check` | exit 0 |

## Scope

**In scope**: `crates/ruxel-agent/src/modules/postgresql.rs` (the three fixes +
new unit tests for the pure functions).

**Out of scope**:
- Changing the `psql`-on-stdin transport, `lit`/`ident`, or the SCRAM password
  idempotence path (all correct).
- The other three privs shapes (database/schema/table) — they work; only add
  `default_privs`.
- On-VM/bless-gate execution (operator; note it in Maintenance).

## Git workflow

- Branch: `advisor/011-pg-idempotence`
- Commit per fix or one `fix(postgresql): idempotent default_privs, full role-flag diff, flag allowlist`.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Implement `default_privs` idempotence via `pg_default_acl`

Replace `"default_privs" => true` (`:392`) with a call to a new
`privs_missing_default(ctx, &port, login_db, role, schema, privs_list)?` that
mirrors `privs_missing_table` but reads `pg_default_acl`:
- Query `pg_default_acl` joined to resolve the role's oid and the target schema
  (`defaclnamespace`), for the relevant `defaclobjtype` (the workload's
  default_privs are on tables — object type `'r'`; confirm against the actual
  playbook usage if available, else handle `'r'` and note the assumption).
- Use `aclexplode(defaclacl)` filtered to the role's grantee oid, collect the
  granted `privilege_type`s (upper-case), and return `true` iff any requested
  priv in `privs_list` is **not** already present — exactly the logic the other
  three `privs_missing_*` use.
- If `defaclacl` is NULL/absent (no default ACL entry yet) treat all requested
  privs as missing → `true`.

Read `privs_missing_table` (`:419+`) first and copy its structure (same
`psql` + `aclexplode` + set-difference shape) so behavior matches.

**Verify**: this is catalog behavior — unit-test what you can (e.g. the SQL
string the function builds, if you factor the query into a pure builder), and
rely on the on-VM bless-gate for the live delta. Add a unit test asserting the
generated query references `pg_default_acl` and `aclexplode` and filters by the
role. `cargo nextest run -p ruxel-agent postgres` → pass.

### Step 2: Compare all role flags, not just SUPERUSER

In `flags_changed` (`:304-320`), map each flag token in the requested
`role_attr_flags` to its `pg_roles` column and compare all of them (the SELECT
already returns `rolsuper,rolcreaterole,rolcreatedb,rolreplication` — add
`rolcanlogin` for LOGIN/NOLOGIN). For each requested flag (e.g. `CREATEDB` vs
`NOCREATEDB`), determine the wanted boolean and compare to the fetched column;
return `true` if **any** differs. Handle the `NO`-prefixed negations (e.g.
`NOSUPERUSER`) explicitly, as the current SUPERUSER logic does.

**Verify**: add a pure unit test for a small helper `wanted_flag(flags,
"CREATEDB") -> Option<bool>` (parse the flag list into per-column desired
booleans) covering `"CREATEDB"` → `Some(true)`, `"NOCREATEDB"` → `Some(false)`,
absent → `None`. `cargo nextest run -p ruxel-agent postgres` → pass.

### Step 3 (SECURITY): Allowlist role_attr_flags before emitting SQL

Add `validate_role_attr_flags(flags: &str) -> Result<(), String>` modeled on
`validate_privs` (`:60-86`): split on `,`/space, uppercase each token, and
reject any token not in the known PG role-attribute set:
`SUPERUSER, NOSUPERUSER, CREATEDB, NOCREATEDB, CREATEROLE, NOCREATEROLE,
INHERIT, NOINHERIT, LOGIN, NOLOGIN, REPLICATION, NOREPLICATION, BYPASSRLS,
NOBYPASSRLS, CONNECTION LIMIT, PASSWORD, VALID UNTIL` — restrict to exactly the
keywords the workload uses (check the playbooks if available; if not, allow the
common boolean attributes above and reject everything else, notably rejecting
any token containing `;`, whitespace-embedded statements, or quotes). Call it in
`user()` **before** `flags_to_sql` is ever used (both the create path `:187` and
the alter path `:204`).

A token like `CONNECTION LIMIT 5` contains a space — decide: either (a) restrict
the allowlist to the boolean attributes the workload actually uses (simplest,
matches closed-surface philosophy — reject `CONNECTION LIMIT`/`PASSWORD`/`VALID
UNTIL` unless the workload uses them), or (b) parse those value-bearing
attributes specially. Prefer (a): allow only the boolean flags, reject the rest
with a clear "outside the closed surface" error, exactly like `validate_privs`.

**Verify**: unit tests: `validate_role_attr_flags("SUPERUSER,CREATEDB")` → Ok;
`validate_role_attr_flags("SUPERUSER; DROP ROLE x")` → Err;
`validate_role_attr_flags("NOLOGIN")` → Ok. **Do not** include a real credential
in any test string. `cargo nextest run -p ruxel-agent postgres` → pass.

### Step 4: Full gates

**Verify**: `cargo fmt --all --check` → 0; `cargo clippy -p ruxel-agent
--all-targets -- -D warnings` → 0; `cargo nextest run` → green.

## Test plan

- Unit tests (pure functions, no DB): `validate_role_attr_flags` accept/reject
  (incl. a `;`-injection reject), `wanted_flag` parsing, and a query-shape
  assertion for `privs_missing_default` (references `pg_default_acl`/`aclexplode`).
- Live catalog behavior (default_privs idempotence, full-flag drift) is verified
  by the **on-VM bless-gate** (operator/fixture, port 40000) — note it; do not
  attempt host contact from CI.

## Done criteria

ALL must hold:

- [ ] `default_privs` idempotence uses `pg_default_acl`/`aclexplode`; the hardcoded `true` is gone
- [ ] `grep -n '"default_privs" => true' crates/ruxel-agent/src/modules/postgresql.rs` → no matches
- [ ] `flags_changed` compares all requested role flags (not only SUPERUSER)
- [ ] `validate_role_attr_flags` rejects any token outside the closed role-flag set (incl. `;`-bearing input) and is called before `flags_to_sql`
- [ ] Pure-function unit tests pass; `cargo nextest run` green; clippy/fmt clean
- [ ] `plans/README.md` row for 011 updated

## STOP conditions

Stop and report if:
- The workload uses a value-bearing role attribute (`CONNECTION LIMIT n`,
  `VALID UNTIL`, `PASSWORD`) that a boolean-only allowlist would reject — report
  it and widen the allowlist deliberately for exactly that attribute (parse its
  value form), do not loosen to "allow anything".
- `pg_default_acl` object type isn't `'r'` for the workload's default_privs
  (e.g. it grants on sequences/functions) — implement the correct `defaclobjtype`
  and report which types are needed.
- The on-VM bless-gate (operator-run) shows a converged `default_privs` rerun
  still `changed` after your fix — the query logic is wrong; report the exact
  SQL and delta.

## Maintenance notes

- The real confirmation is the on-VM three-way bless-gate
  (`tools/fixtures/bless-gate.sh`) on a PG fixture: `ruxel apply` → `ruxel`
  rerun `changed=0` → Ansible bless `changed=0`, for a play exercising all four
  privs shapes. This fix makes the *pinned* SEMANTICS claim actually true; flag
  for the operator to re-run that gate.
- Reviewer: Step 1 is the load-bearing fix — verify the `pg_default_acl` query
  filters to the *explicit* role grant (not PUBLIC/inherited), matching the
  documented aclexplode rationale at `postgresql.rs:412-417`.
- After this, `SEMANTICS.md`/`GOAL.md`'s "all four shapes idempotent" claim is
  finally accurate (plan 002 should stop treating it as an oversold doc claim).

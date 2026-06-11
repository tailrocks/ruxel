# SEMANTICS — What Ansible Does With These Files, Exactly

Status: normative spec, 2026-06-11. This document defines what "drop-in"
means: for every construct that appears in the ChainArgos playbooks, the
behavior Ansible (core 2.21) gives it today, which ruxel must reproduce.
Items marked **⚠ verify** are subtleties to be pinned down empirically in
the M1/M3 parity harness (see [PLAN.md](PLAN.md)) before the corresponding
code is considered done — never assumed.

Scope discipline: this spec covers exactly the surface extracted from the 16
playbooks on 2026-06-11 (param-level matrix in §6). Anything outside it is
out of scope by definition ([WORKLOAD.md](WORKLOAD.md)).

---

## 1. File and play model

- A playbook file is a YAML list of plays. The workload uses one play per
  file with keys: `name`, `hosts`, `become`, `vars`, `pre_tasks`, `tasks`,
  `handlers`.
- `hosts:` is an inventory pattern; effective hosts = pattern ∩ `--limit`.
  The workload uses literal hostnames and the group `nodes`.
- Inventory is INI: one group, `ansible_ssh_host` (connection address) and
  `ansible_ssh_user` (login user) per host. `ansible_python_interpreter:
  auto_silent` appears in every play's vars and is parsed-and-ignored by
  ruxel (no Python anywhere).
- `become: yes` at play level: every task runs privileged. Login user is
  already root on all hosts, so `become` is effectively a no-op wrapper,
  **except** `become_user: postgres` (46 uses), which must execute the task
  as the `postgres` user (uid/gid + supplementary groups + HOME/USER env).
  **⚠ verify**: environment Ansible sets under become_user (it does not run
  a login shell; `HOME` handling differs between `sudo` flags).

## 2. Variables and templating

- Template language: Jinja2, `{{ }}` / `{% %}`, with Ansible's **native
  types** semantics: a template that evaluates to a list/dict/bool/int
  yields that object, not its string form (matters for `loop:
  "{{ list_var }}"` — 20 uses — and `| bool`).
- Evaluation is **lazy**: play `vars` are unevaluated definitions; rendering
  happens each time a value is referenced in a task context.
- Variable sources in the workload, lowest→highest effective precedence:
  play `vars` → `set_fact` results → `register` results. Facts live under
  both `ansible_*` top-level names and `ansible_facts[...]`.
- **Lookups** run on the controller at render time, every time:
  `lookup('community.general.onepassword', '<item>', field=, vault=,
  section=)` (52 uses) and `lookup('pipe', 'op read "op://…"')`.
  **Specified deviation:** ruxel memoizes each distinct lookup invocation
  once per run and resolves all of them concurrently before execution.
  Ansible's literal behavior (re-running `op` per reference) is treated as
  an implementation accident, not a semantic: within one run the operator's
  intent is one consistent secret snapshot. This is the only deliberate
  behavioral deviation in this spec.
- Jinja constructs in use (closed list): filters `default`, `bool`,
  `urlencode`, `map`, `list`, `length`, `hash('sha256')`, `subelements`;
  attribute/index access on registered results (`x.stat.exists`,
  `ch_ready.rc`); comparisons and boolean operators in `when`/`until`;
  loop over `register` results (`item.item`, `item.stat`). MiniJinja covers
  the core; `subelements` and Ansible-flavored `hash` need custom filter
  implementations. **⚠ verify**: M1 harness renders every template and every
  inline expression in the repo through ansible-core's Templar and through
  ruxel's engine and diffs byte-for-byte — that harness, not this list, is
  the completeness guarantee.
- Facts consumed (complete list): `ansible_default_ipv4.interface`,
  `ansible_facts['distribution_release']`, `ansible_architecture`. Ruxel's
  agent supplies exactly these (plus trivially cheap extras like hostname)
  in its handshake; no general fact system.

## 3. Task execution pipeline (linear strategy, one host)

Per task, in order — ruxel must preserve this pipeline observably:

1. Inherit play keywords (`become`, play `vars`).
2. **`when`**: bare Jinja boolean expression; a list of strings = AND of
   all. Evaluated **per loop item** when `loop` is present. Skipped tasks
   produce a "skipped" result; a skipped task never notifies or registers
   changed. **⚠ verify**: registered var shape on skip (dict with
   `skipped: true`).
3. **`loop`**: value is either a literal list or `"{{ var }}"` (native-type
   list). Each iteration binds `item`; `loop_control.label` only affects
   display. Result registered from a looped task is a dict with `results:
   [per-item results]`, aggregate `changed` = any item changed, aggregate
   failed = any failed.
4. Render module params with current scope (+`item`).
5. **check mode** (`--check`): modules predict and report; `shell`/
   `command` are **skipped** in check mode unless the task sets
   `check_mode: no` (6 uses — those run for real even under `--check`,
   deliberately: they feed `readlink`-style data into later templating).
   `check_mode: no` on a task means "always execute, even in check runs."
6. Execute module (under `become_user` if set, with task `environment`
   merged into the process env — keys in use: `DEBIAN_FRONTEND`,
   `CLICKHOUSE_PASSWORD`).
7. **`changed_when`** (26 uses, mostly `false`): overrides the module's
   changed flag; expression may reference the just-produced result.
   **`failed_when`** (1 use: `rc not in [0, 1]`): replaces default failure
   detection. Order: failed_when is evaluated before changed_when matters
   for reporting; both see the raw result. **⚠ verify** exact interaction
   when both present (not co-present in the workload — then irrelevant).
8. **`register`**: bind result dict. Command/shell results carry `rc`,
   `stdout`, `stderr`, `stdout_lines`, `changed`, `failed`; `stat` carries
   `stat.{exists,isdir,islnk,…}`; `slurp` carries base64 `content`.
   Registered even on failure and on skip.
9. **`ignore_errors: yes`** (7 uses): failure is recorded but execution
   continues; play recap counts it.
10. **`until`/`retries`/`delay`** (1 use: `until: ch_ready.rc == 0`,
    retries 10, delay 3): re-execute the module until the expression on the
    registered result is true; at most `retries` attempts spaced `delay`
    seconds; final result carries `attempts`. Failure after exhaustion =
    task failure.
11. **`no_log: true`** (6 uses): result values, rendered params, and loop
    item display are redacted in all output and logs (including errors).
12. **`notify`** (7 uses): if final changed == true, add named handler(s)
    to the play's notified set (deduplicated by handler name).

Failure of a task (not ignored, not rescued) stops execution **for that
host**; remaining hosts continue (no `any_errors_fatal`/`serial` in the
workload).

## 4. Blocks, handlers, sections, tags, CLI surface

- **`block`/`rescue`** (3 blocks): tasks grouped; on the first failing task
  in `block`, jump to `rescue`; if rescue completes, the play continues and
  the host is not marked failed. (`always` not used.) Keywords on the block
  (become, when, tags) are inherited by contained tasks.
- **`pre_tasks`** run before `tasks` (1 use: secret-validation asserts on
  sentry). Handlers notified in pre_tasks flush before `tasks` in Ansible;
  the workload never notifies from pre_tasks, so the only flush point that
  matters is end-of-play. **Handlers**: run once each at flush, in
  handler-definition order, only if notified by a changed task.
- **`tags`** (sentry only: `always`, `sentry`, `velnor`, `garm-remove`):
  with `--tags X`, run tasks tagged X plus tasks tagged `always`; all
  others report skipped. Without `--tags`, everything runs. Tags on a block
  apply to its tasks.
- **CLI surface to preserve** (shape, not flag-for-flag parity):
  `-i hosts.ini`, `--limit <pattern>`, `--check`, `--diff`,
  `--tags <list>`, `-e KEY=VALUE` not used. Exit code 0 = success
  (regardless of changed), non-zero = any host failed.
- **`--diff`** output: file-content before/after for template/copy/
  lineinfile/replace/blockinfile/file; ruxel renders unified diffs with
  secrets redacted under `no_log`.
- **Interactive `pause`** (1 use, with `prompt`): blocks awaiting operator
  Enter on the controller TTY; must keep working (Sentry manual bootstrap),
  including under ruxel's streaming execution.

## 5. Connection-level semantics being replaced

Ansible specifics ruxel replaces outright (not part of the observable
contract): AnsiballZ payload upload, per-task SSH exec + temp dirs, remote
Python discovery, SFTP-per-task. The observable contract is **only**: tasks
see a root (or become_user) process on the target with the rendered params,
in the order and under the conditions above, and report results with the
fields above.

## 6. Module semantics (normative, param-scoped)

For each module: exactly the parameters in use (counts from 2026-06-11
extraction), the idempotence check Ansible performs, the change action, and
check-mode behavior. Ruxel implements **these params only**; an unknown
param in a future playbook edit must be a hard parse error (fail loud, not
ignore — that is what closed spec means).

### Files & content

- **`file` (46)** — params: `path/dest/src`, `state` (**exactly three
  values in use**: `directory` ×30, `absent` ×11, `link` ×1), `owner`,
  `group`, `mode`, `recurse`.
  Check: lstat path; directory = exists+isdir+owner/group/mode (recurse:
  applies down-tree); link = symlink target equals `src`; absent = not
  exists. Change: mkdir -p / ln -sf / rm -rf / chown+chmod. Check-mode:
  predict. **⚠ verify**: mode given as string `"0700"` octal handling.
- **`copy` (35)** — `src` (controller-relative file) or `content`, `dest`,
  `owner/group/mode`, `force`. Check: SHA1(content) vs SHA1(dest) +
  attrs; `force: no` = only copy if dest missing. Diff supported.
- **`template` (41)** — `src` (Jinja file), `dest`, `owner/group/mode`.
  Render with full var scope (incl. 1P-derived vars), then byte-compare to
  dest. Trailing-newline and `keep_trailing_newline=True` behavior must
  match Ansible's template module defaults. **⚠ verify** against all 22
  real templates in M1 (byte-for-byte).
- **`lineinfile` (15)** — `path`, `regexp`, `line`, `state`.
  present: if regexp matches a line → replace last matching line with
  `line`; else append `line` at EOF; absent: delete matching lines.
  Idempotent if a line already equals `line` **⚠ verify** (exact
  Ansible rule: if `line` already present unchanged even if regexp also
  matches elsewhere — pin with fixture tests; fstab/pam edits depend on it).
- **`replace` (3)** — `path`, `regexp` (multiline), `replace`. Changed iff
  substitution alters content.
- **`blockinfile` (2)** — `path`, `block`, `create`, `owner/group/mode`.
  Managed block between default markers (`# BEGIN/END ANSIBLE MANAGED
  BLOCK`); insert at EOF if absent; replace content between markers.
- **`stat` (11)** — `path`, `follow`. Read-only; returns `stat.*` fields
  used: `exists`, plus disk checks. Never changed.
- **`slurp` (1)** — `src`; returns base64 `content`. Read-only.
- **`get_url` (5)** — `url`, `dest`. If dest exists → unchanged (no
  checksum given, `force` not set); else download to dest. **⚠ verify**
  default timeout/redirect behavior is irrelevant here; confirm
  dest-exists-short-circuit matches (it does when force=no default).

### Packages & repos

- **`apt` (24)** — `name` (str or list), `state` (`present` ×17,
  `latest` ×2 — no other values),
  `update_cache`, `upgrade: dist`, `autoremove`, `force`. Check: dpkg
  status for each name (present = installed at any version; latest =
  installed AND no candidate newer per apt policy). Change: `apt-get
  install/upgrade` with `DEBIAN_FRONTEND=noninteractive`. `update_cache`
  refreshes lists; **⚠ verify** when Ansible reports `changed` for
  update_cache-only invocations and for `upgrade: dist` with nothing to do.
  Ruxel batching rule (§ ARCHITECTURE 5.3) must not alter per-task reported
  status.
- **`apt_repository` (6)** — `repo` (deb line), `filename`, `state`,
  `update_cache`. Check: exact sources line present in
  `/etc/apt/sources.list.d/<filename>.list`. Change: write file + cache
  refresh.

### Services & kernel

- **`systemd` (21)** — `name`, `state` (`started` ×5, `stopped` ×5,
  `restarted` ×3 — exactly these), `enabled`, `daemon_reload`. Check: unit ActiveState/UnitFileState via
  systemd. `restarted` is **always a change** (action, not state).
  daemon_reload: executes reload; **⚠ verify** its changed semantics
  (Ansible reports changed when daemon_reload runs? pin in fixtures).
- **`service` (8)** — `name`, `state` (`started` ×4, `restarted` ×4),
  `enabled`. On these hosts resolves to systemd; same semantics as above.
- **`sysctl` (10) / `ansible.posix.sysctl` (6)** — `name`, `value`,
  `state`, `sysctl_set`, `reload`, `sysctl_file`. Check: value in target
  file (and live value when sysctl_set). Change: write file + `sysctl -w` /
  `--system` reload. Value comparison is **string-normalized** (whitespace
  in multi-value keys like `vm.nr_hugepages` vs `net.ipv4.ip_local_port_range`)
  **⚠ verify** normalization rules.
- **`community.general.timezone` (1)** — `name: UTC`; timedatectl.

### Storage

- **`community.general.lvg` (6)** — `vg`, `pvs` (list of /dev/disk/by-id
  paths). Check: VG exists with exactly these PVs (**⚠ verify**: Ansible's
  behavior when VG exists with a subset of pvs — it extends; with extras —
  it reduces? pin before implementing; the drive-add workflow in the
  AGENTS.md depends on "add disk to list, re-run").
  Change: pvcreate + vgcreate/vgextend.
- **`community.general.lvol` (6)** — `vg`, `lv`, `size` (incl.
  `+100%FREE`), `resizefs`. Check: LV exists; size semantics: `%FREE` form
  is only meaningful at creation/extension — **⚠ verify** idempotence rule
  for `size: +100%FREE` on an already-full LV (must be no-change, not
  error; Ansible handles via lvextend rc/output inspection).
- **`filesystem` (6)** — `dev`, `fstype` (`xfs` ×5, `ext4` ×1),
  `resizefs`. Check:
  blkid fstype on dev; create only if absent; resizefs grows fs to device.
- **`ansible.posix.mount` (6)** — `path`, `src` (UUID=…), `fstype`,
  `opts`, `state: mounted`. Check: fstab line (normalized fields) +
  actually mounted. Change: write fstab + mount. **⚠ verify** fstab
  field normalization (opts order, dump/pass defaults) so reruns are
  no-change.

### Users, keys, firewall

- **`user` (5)** — `name`, `uid`, `group`, `groups`, `append`, `home`,
  `create_home`, `shell`, `comment`, `system`, `state` (only one explicit
  use: `absent`, the GARM cleanup; the other four default to present),
  `remove`. Check: passwd/group entries field-by-field. Change:
  useradd/usermod/userdel.
- **`group` (3)** — `name`, `gid`, `state`. getent group check.
- **`authorized_key` (1)** — `user`, `key` (1P-sourced), `state`. Check:
  exact key line (comment-insensitive matching on key material **⚠
  verify**) in `~user/.ssh/authorized_keys`.
- **`iptables` (8)** — `chain`, `protocol`, `destination`,
  `out_interface`, `jump`, `comment`, `policy`, `ip_version`. Check: rule
  spec present (iptables -C equivalent); policy: current chain policy.
  Change: append rule / set policy. Note rule-order sensitivity is handled
  by the playbook (DOCKER-USER insert order workaround) — ruxel must
  preserve append vs insert semantics exactly as the module does.

### Git & databases

- **`git` (10)** — `repo`, `dest`, `version` (branch), `update`
  (false = clone-only-if-absent), `force`, `accept_hostkey`. Check:
  dest/.git exists; if update: compare remote HEAD SHA for version vs local
  (network fetch — see ARCHITECTURE on probe classes). Changed = SHA
  before≠after (or fresh clone). `force`: discard local modifications.
- **`community.postgresql.postgresql_db` (17)** — `name`, `owner`,
  `state`, `login_user: postgres`, `login_port: 40000`. Check: pg_catalog
  (datname, datdba). Change: CREATE DATABASE / ALTER OWNER.
- **`community.postgresql.postgresql_user` (7)** — `name`, `password`,
  `role_attr_flags`, `state`, login params. Check: pg_roles + attr flags;
  password idempotence: Ansible compares against stored SCRAM-SHA-256
  verifier **⚠ verify** (it hashes and compares; must not report changed
  every run — pin exact rule; this is a known subtlety).
- **`community.postgresql.postgresql_privs` (20)** — `role`, `privs`,
  `type` (**exactly four shapes in use**: `database` ×3, `schema` ×3,
  `table` ×7, `default_privs` ×7), `objs`, `schema`, `login_db`, `state`.
  Check: current ACL vs requested grant set; changed only on real ACL
  delta. The subtlest module in the set — implement against pg_catalog ACL
  parsing with fixture tests per grant shape used in the playbooks.
- **`community.postgresql.postgresql_schema` (1)** — `name`, `login_db`,
  `state`; pg_namespace check.

### Commands & control

- **`command` (40)** — free-form or `cmd`/`argv`, `chdir`. No shell;
  argv exec. Failure = rc≠0. Always "changed" unless `changed_when`.
  Check-mode: skipped (unless task `check_mode: no`).
- **`shell` (50)** — free-form via shell; `args`: `executable` (44 — the
  only value in use is `/bin/bash`), `chdir` (38), `creates`
  (7: skip task entirely if path exists — evaluate before execution, report
  skipped/ok **⚠ verify** reported status when creates-guard fires:
  Ansible reports `ok`/`skipped`? pin it). Env merging per task
  `environment`.
- **`assert` (7)** — `that` (list of expressions), `fail_msg`. Controller-
  side evaluation in ruxel (pure expression over vars/facts).
- **`fail` (2)** — `msg`; unconditional failure (guarded by `when`).
- **`debug` (1)** — `msg`; print rendered value.
- **`set_fact` (1)** — sets host var for the rest of the play (the Sentry
  bootstrap marker flag).
- **`pause` (1)** — `prompt`; interactive, §4.

## 7. Output contract

Ruxel's default human output mirrors Ansible's shape (task header lines,
`ok/changed/skipped/failed` per host, recap with counts) so existing
operator habits and any log-greps keep working, plus: per-task wall-time,
probe-vs-executed annotation (`ok (verified: ledger)` vs `ok (checked)`),
and a machine mode `--output json` (one JSON event per line, stable
schema). `no_log` redaction applies everywhere including diffs and JSON.

# The Workload — Closed Compatibility Spec

This document is the **complete, closed specification** of what ruxel must
execute. It is an exhaustive inventory of the ChainArgos `ansible-configs`
directory (16 playbooks, 452 named tasks, 3,661 lines of YAML, 42 config
files, 6 hosts) as of 2026-06-11. Ruxel implements everything in this
document and nothing outside it. When the playbooks change, this spec changes
with them — not the other way around.

Source of truth: `ChainArgos/java-monorepo/ansible-configs/`.

## 1. Module inventory (29 distinct modules)

### Built-in modules (short names, per repo convention)

| Module | Uses | Notes |
|---|---|---|
| `shell` | 54 | incl. 36× `eval "$(mise activate bash)" && ./containerctl.main.kts restart <node>` |
| `file` | 46 | directories, symlinks, permissions |
| `command` | 40 | `git lfs install`, `mise trust`, `readlink -f` disk resolution |
| `group` | 39 | user group management |
| `copy` | 35 | static file deployment |
| `apt` | 24 | package install/upgrade |
| `systemd` | 21 | daemon_reload, enable, restart |
| `lineinfile` | 15 | fstab swap comments, pam_limits |
| `stat` | 11 | disk existence, bootstrap markers |
| `git` | 10 | repository clones |
| `service` | 8 | older-style service calls |
| `iptables` | 8 | firewall rules (pegasus) |
| `replace` | 6 | regex file edits |
| `user` | 6 | system users (chainargos-backup, airflow, looker) |
| `get_url` | 5 | GPG key downloads |
| `template` | 41 | Jinja2-rendered config files |
| `sysctl` | 10 | kernel tuning (also via `ansible.posix.sysctl`) |
| `blockinfile` | 2 | SSH config blocks |
| `filesystem` | 6 | XFS/ext4 formatting |
| `pause` | 1 | manual Sentry bootstrap confirmation |
| `debug` | 1 | diagnostics |
| `slurp` | 1 | read Sentry bootstrap marker |
| `assert` | n/a | pre-task validation (arch checks, token length) |
| `authorized_key` | 1 | backup user SSH key |

### Collection modules (FQCN, per repo convention)

| Module | Uses | Notes |
|---|---|---|
| `community.postgresql.postgresql_privs` | 20 | GRANTs |
| `community.postgresql.postgresql_db` | 17 | 9 DBs on nova, 5+ on titan |
| `community.postgresql.postgresql_user` | 7 | 12+ users total |
| `community.postgresql.postgresql_schema` | 1 | |
| `community.general.lvg` | 6 | VGs: blockchain, data, backup, clickhouse-hot |
| `community.general.lvol` | 6 | LVs, `+100%FREE` |
| `community.general.timezone` | 1 | UTC |
| `ansible.posix.sysctl` | 6 | with `sysctl_file` |
| `ansible.posix.mount` | 6 | UUID-based `src`, fstab persistence |

Convention (from repo AGENTS.md): built-ins use short names, collection
modules keep explicit FQCN prefixes. Ruxel must accept both spellings it
encounters in these files.

## 2. Language features

| Feature | Count | Representative use |
|---|---|---|
| `become: yes` (play level) | 16 | all plays |
| `become_user` | 46 | `postgres` for postgresql_* tasks |
| `handlers` + `notify` | 2 / 7 | docker daemon.json restart; velnor daemon restarts |
| `tags` | 4 | `always, sentry, velnor, garm-remove` (setup-sentry.yml) |
| `when` | 20 | stat-result conditions |
| `register` | 44 | stat loops, command output |
| `loop` | 29 | iptables IP ranges, disks, DBs, users |
| `loop_control` | 6 | `label:` |
| `changed_when` | 26 | mostly `false` for idempotent commands |
| `ignore_errors` | 7 | iptables chain may exist |
| `failed_when` | 1 | rc not in [0, 1] |
| `check_mode: no` | 6 | readlink during drive init |
| `block` / `rescue` | 5 | Sentry config blocks |
| `vars` (play-level, inline) | 16 | with 1Password lookups |
| `set_fact` | 1 | SHA256 password hashing |
| `no_log: true` | 3 | secret-bearing tasks |
| `pre_tasks` | 1 | secret validation asserts |
| `gather_facts` | default on | see Facts subset below |
| Not used | — | `serial`, `delegate_to`, `run_once`, `any_errors_fatal`, roles, includes/imports, vault files |

### Facts subset actually consumed

Only these fact paths are referenced — a full fact system is unnecessary:

- `ansible_default_ipv4.interface` (iptables rules)
- `ansible_facts['distribution_release']` (Docker repo URL)
- `ansible_architecture` (sentry pre-task assert: must be x86_64)

### Templating

- Jinja2 in task params, `when`/`changed_when`/`failed_when` bare
  expressions, and 22 templated config files (`.env`, `config.yml`,
  `sentry.conf.py`, `users.xml`).
- Filters/functions observed: `default()`, lookup plugins (below), SHA256
  hashing via `set_fact`.
- 20 further config files are pure static (deployed via `copy`).

### Lookup plugins (the entire secrets story)

- `community.general.onepassword` — 40+ distinct lookups
  (vault `ChainArgos`; fields like `password`, `private key`, sections).
- `lookup('pipe', 'op read "op://ChainArgos/..."')` — SSL certs, GCP key.
- No ansible-vault anywhere. No plaintext secrets in the repo.
- Requires 1Password CLI (`op`) on the controller machine only.

## 3. Inventory shape

`hosts.ini`, INI format, one group:

```ini
[nodes]
pegasus           ansible_ssh_host=<ip> ansible_ssh_user=root
delorean          ansible_ssh_host=<ip> ansible_ssh_user=root
titan             ansible_ssh_host=<ip> ansible_ssh_user=root
sentry            ansible_ssh_host=<ip> ansible_ssh_user=root
postgresql-nova   ansible_ssh_host=<ip> ansible_ssh_user=root
clickhouse-selene ansible_ssh_host=<ip> ansible_ssh_user=root
```

- 6 Hetzner dedicated servers, Debian 12, root SSH, port 22, key auth,
  no bastion. All production.
- Every playbook sets `ansible_python_interpreter: auto_silent` (a Python-ism
  ruxel parses and ignores).

## 4. Playbooks

| Playbook | Tasks | Target | Purpose |
|---|---|---|---|
| `install-base.yml` | 26 | all | base packages, zsh/oh-my-zsh, starship, nushell, mise, GraalVM, Rust, cargo tools, holla |
| `install-docker.yml` | 9 | all | Docker CE + daemon.json (handler restart) |
| `update-packages.yml` | 4 | all | apt dist-upgrade + mise upgrade |
| `upgrade-debian.yml` | 4 | all | Hetzner mirror sources |
| `init-pegasus-drives.yml` | 7 | pegasus | 7×NVMe → `blockchain` VG → XFS |
| `init-titan-drives.yml` | 7 | titan | 2×NVMe → `data` VG → XFS |
| `init-postgresql-nova-drives.yml` | 7 | postgresql-nova | 3×NVMe → `data` VG → XFS |
| `init-clickhouse-selene-drives.yml` | 12 | clickhouse-selene | two-tier: `data` (28 TiB) + `clickhouse-hot` VGs |
| `init-delorean-drives.yml` | 7 | delorean | 2×22TB SATA → `backup` VG → ext4 |
| `setup-pegasus.yml` | 33 | pegasus | mounts, iptables (56 rules), repos, env files, backup user |
| `setup-delorean.yml` | 35 | delorean | users, mounts, repos, 18 env templates, SSL via 1P |
| `setup-titan.yml` | 60 | titan | PostgreSQL 18, pg_parquet, kernel tuning, DBs/users |
| `setup-postgresql-nova.yml` | 65 | postgresql-nova | largest: PostgreSQL 18, 9 DBs, 8 users, kernel tuning |
| `setup-clickhouse-selene.yml` | 40 | clickhouse-selene | ClickHouse, two-tier storage, XML configs, DBs/users |
| `setup-sentry.yml` | 50 | sentry | Sentry self-hosted + 3 Velnor runner daemons, tags |
| `restart-blockchain-nodes.yml` | 36 | pegasus | 36 identical shell restarts via containerctl |

## 5. Invocation surface to preserve

```bash
ruxel ... -i hosts.ini --limit <host> <playbook>.yml            # target one host
ruxel ... -i hosts.ini <playbook>.yml                            # all hosts
ruxel ... --check --diff                                         # dry run
ruxel ... --tags sentry|velnor|garm-remove                       # sentry only
```

(`ANSIBLE_LOCAL_TEMP` workaround becomes unnecessary.) Tags must parse and
work for setup-sentry.yml, but the design goal is to make full reruns cheap
enough that tag-selection stops being a performance tool.

## 6. Known pain points in the current setup

These are measured/observed properties of the workload that the design must
answer (see [DIRECTION.md](DIRECTION.md)):

1. **Converged rerun ≈ full-cost rerun.** Ansible re-verifies every task
   remotely with a fresh Python payload each time; ~15 minutes to do nothing.
2. **40+ serial 1Password lookups**, each its own `op` subprocess, paid on
   every run before any task executes.
3. **No intra-host parallelism.** With `--limit <one host>` (the common
   case), Ansible's host-level forks buy nothing; 65 tasks run strictly
   sequentially.
4. **Duplication**: 36 identical restart tasks, 18 near-identical env-file
   tasks, the same LVM/mount pattern in 4 files, the same SSH-key pattern in
   6 — all paying full per-task overhead each.
5. **Manual-step orchestration** (Sentry bootstrap pause + marker file)
   must keep working.
6. **Workaround scar tissue**: DOCKER-USER iptables ordering, mise
   activation prefix in every shell task, netfilter-persistent always-run.

## 7. Acceptance bar

Ruxel is "done" for a playbook when running it against a converged test
target produces the same end state as `ansible-playbook` does, byte-for-byte
where observable (files, packages, units, mounts, DB objects), with
`plan` output equivalent to `--check --diff`, while meeting the performance
goals in [VISION.md](VISION.md). The production servers are never part of
that verification loop.

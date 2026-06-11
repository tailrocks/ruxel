# RESTORE — where ruxel stands and what's next

Handoff snapshot to resume work after a break. Authoritative operational
runbook is still `GOAL.md`; this file is the quick "what happened / where
we are / what's next" restore point. Date: 2026-06-12. Repo HEAD: `1411ba0`
(all work below is committed + pushed to `tailrocks/ruxel` main, CI green).
Hetzner `ruxel-fixtures` project is **empty** (every fixture reaped).

---

## 1. One-line status

**ruxel is feature-complete** — every module, the convergence ledger, the
full drop-in CLI, and real secret resolution are built and verified. **6 of
16 workload playbooks are gated three-way** (ruxel apply → ruxel rerun →
real Ansible blesses the state, all at parity). Converged-run speed proven:
**ruxel 1.04s vs Ansible 15.16s (14.6×)**. What remains is **verification
breadth** (the 6 `setup-*` + drive-variant playbooks) and the rest of the
M5 benchmark suite — not missing features.

Milestones: **M0 ✅ M1 ✅ M2 ✅ M3 ✅** (modules + ledger + plan/apply) ·
**M4 ~** (full module set done; remaining = setup-* gate coverage) ·
**M5 ~** (headline converged benchmark done; fresh-provision + parallel +
profile pending) · **M6** = operator-driven production pilot (untouched, by
design).

---

## 2. What is DONE and verified

**All 36 closed-surface modules** execute on real x86_64 Debian fixtures:
- files/content: file, copy, template, lineinfile, replace, blockinfile, stat, slurp
- packages/repos: apt, apt_repository, get_url
- services/kernel: systemd, service, sysctl (both spellings), community.general.timezone
- storage: community.general.lvg, lvol, filesystem, ansible.posix.mount (proven on real Hetzner volumes)
- users/keys/fw: user, group, authorized_key, iptables
- vcs: git
- postgresql ×4: postgresql_db/user/schema/privs — both subtleties pinned
  (SCRAM password idempotence by re-deriving the StoredKey; privs
  idempotence on the *explicit* ACL via aclexplode, not has_*_privilege)
- control/controller-side: command, shell, debug, assert, fail, set_fact, pause
- become_user (postgres etc.) via runuser wrapper

**Convergence ledger** (the "seconds" promise, ARCHITECTURE §6): per-host
`/var/lib/ruxel/ledger/ledger.json`, keyed by a blake3 of task identity +
rendered params; converged tasks replay cached fingerprints (File/Pkg/Unit/
SysctlKV) without invoking the module. Honesty rule enforced (always-execute
modules never cached). `--no-cache` bypasses.

**Drop-in CLI:** `ruxel plan|apply -i hosts.ini [--limit] [--check]
[--diff] [--tags] [--output human|json] [--dry-secrets] [--no-cache]
playbook.yml`.

**Secrets:** real `op` resolver (verified against the `ruxel-test` vault) +
`--dry-secrets` deterministic-fakes test path. 52 lookups memoized to a
handful of `op` calls per run.

**Transport / fidelity:** openssh ControlMaster (own tokio::process),
content-addressed agent upload, orphan guard; render parity = 242
expressions + 41 templates byte-identical to ansible-core 2.21 (offline CI
gate). Runtime semantics (loop/when/register/until/no_log/changed_when)
golden-pinned from real 2.21 captures.

**Playbooks gated three-way** (`tools/fixtures/bless-gate.sh`): `update-
packages`, `upgrade-debian`, `install-docker`, drives (lvg/lvol/filesystem/
mount), postgresql (db/user/schema/privs), `install-base` (39 tasks, holla
installed from the live repo). "Parity" not "zero": a converged rerun still
reports the bare `command`/`shell` tasks that have no `changed_when` —
Ansible reports those changed every run too, so equal changed-sets = parity.

**M5 headline benchmark** (`docs/benchmarks/converged-noop.md`): install-
docker converged no-op, cpx22 fixture, best of 3 — ruxel+ledger **1.04s**,
ruxel --no-cache 4.04s, ansible **15.16s**.

**holla-apt.tailrocks.com is LIVE** (operator-directed side task, done):
fixed the deployment end-to-end (4 merged workflow PRs across tailrocks/
holla + tailrocks/holla-apt — heredoc indent, arm64 --no-strip, cargo-deb
mise resolution, binary-keyring NO_PUBKEY). Verified: Release 200, holla
0.4.2 in amd64+arm64 Packages, gpg served. install-base + setup-* can now
install holla.

---

## 3. THE NEXT STEP (blocked on operator)

**Drop a read-only deploy key for the ChainArgos private repos
(`java-monorepo`, `blockchain-nodes`) into the `ruxel-test` 1Password
vault.** Suggested item: title `chainargos-deploy SSH`, field `private key`
(+ `public key`). This is the only blocker for the 6 `setup-*` gates — they
`git clone git@github.com:ChainArgos/java-monorepo.git`, and `--dry-secrets`
supplies a fake key that can't authenticate (Ansible fails identically).
Same class as the holla-apt token. ~2 min. Tell me the item name and I run
the setup-* sweep autonomously.

Two gate facts the sweep needs (already understood):
1. setup-* assume `install-base.yml` ran first (git/mise/base) — gate
   sequence is: apply install-base, then the setup-* on the same fixture.
2. setup-* `hosts:` are literal prod hostnames — gate via a `hosts: all`
   copy kept **in the workload dir** (so `config/` src paths resolve):
   `sed 's/^  hosts: <name>/  hosts: all/' <pb> > <configs-dir>/.ruxel-gate.yml`

---

## 4. Autonomous work available WITHOUT the key (lower value)

- Rest of M5: fresh-provision wall-clock; per-task `profile_tasks`-style
  breakdown; 65-task setup-postgresql-nova converged number (partial w/o
  key — stops at the private clone).
- 6-hosts-parallel benchmark — **needs the multi-host transport fix first**
  (documented known issue: a 2nd sequential connect in one process stalls;
  see the header comment in `crates/ruxel/src/transport.rs`). Revisit before
  any parallelism.
- Drive-variant gates (init-{titan,delorean,pegasus,nova,selene}-drives) —
  need volumes (operator pre-approved), no key. Storage shape already proven.
- `--diff` extension to lineinfile/replace/blockinfile (copy/template done).
- apt adjacency batching + content-addressed blob channel (perf, ARCHITECTURE
  §5.3 / §1) — optimizations, not parity.

---

## 5. How to resume (commands + locations)

**Build:**
- controller: `cargo build --release -p ruxel-cli` → `target/release/ruxel`
- agent (x86_64 fixtures): `mise exec -- cargo zigbuild --target
  x86_64-unknown-linux-musl -p ruxel-agent --release`
- agent (local arm64 OrbStack VM): `… --target aarch64-unknown-linux-musl …`
- gates before every commit: `cargo fmt --all --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`
  (workload tests need `RUXEL_WORKLOAD_DIR=~/Projects/ChainArgos/
  java-monorepo/ansible-configs`).

**Fixtures (Hetzner, `ruxel-fixtures` hcloud context — active):**
- create: `RUXEL_FIXTURE_TYPE=cpx22 tools/fixtures/create.sh <suffix>`
  (default cpx12@sin; cpx22 for docker/PG-class; volumes auto-reaped)
- add volumes: `hcloud volume create --name ruxel-fixture-vol-N --size 10
  --server <name> --label ruxel=fixture` (appear at
  /dev/disk/by-id/scsi-0HC_Volume_*)
- destroy/reap: `tools/fixtures/destroy.sh <name>` then
  `tools/fixtures/reap.sh` — **always reap at session end** (operator: no
  lingering paid resources). Confirm empty: `hcloud server list`,
  `hcloud volume list`.
- local inner-loop VM (free): OrbStack `ruxel-deb` (arm64 Debian 12),
  recreate with `orb create -a arm64 debian:bookworm ruxel-deb`.

**Gate a playbook (three-way parity):**
`tools/fixtures/bless-gate.sh root@<fixture-ip> <keyfile> <agent-bin>
<playbook> "" dry`  (the trailing `dry` = dry-secrets both sides; omit for
secret-free playbooks). keyfile printed by create.sh as RUXEL_FIXTURE_KEY.

**Safety rules (GOAL.md, absolute):** never touch the six production hosts
(pegasus, delorean, titan, sentry, postgresql-nova, clickhouse-selene / any
IP in the real hosts.ini). Only targets = self-created `ruxel-fixtures`
VMs. Verify the target IP ≠ all six prod IPs before the first remote
command. Real secrets never enter fixtures/captures/commits (synthetic
`ruxel-test` vault + dry-secrets only).

**Credentials (pointers, values never in repo):**
- hcloud token: active context `ruxel-fixtures` (~/.config/hcloud/cli.toml);
  1P backup `ChainArgos / ruxel Hetzner Cloud`. Operator may rotate.
- 1P service account (R/W on `ruxel-test`): `~/.config/ruxel/op-ci.env`
  (`OP_SERVICE_ACCOUNT_TOKEN`); also GH secret on tailrocks/ruxel; 1P backup
  `ChainArgos / ruxel CI service account`. For op reads:
  `set -a; source ~/.config/ruxel/op-ci.env; set +a`.

**Oracle (pinned ansible 2.21):** `tools/oracle/` (uv venv). Capture a real
run: `tools/oracle/capture_fixture.sh <ip> <key> <playbook> <name>`
(prepend `RUXEL_DRY_SECRETS=1` for secretful playbooks → fake op lookups).
Real galaxy collections live in `tools/oracle/galaxy/` (gitignored;
reinstall: `cd tools/oracle && uv run ansible-galaxy collection install -r
galaxy-requirements.yml -p galaxy`).

---

## 6. Operator to-dos still open (non-blocking)

- Revoke the old read-only `ruxel-ci` 1P service account (replaced by the
  R/W one). *(Operator said this is done.)*
- Rotate the Hetzner token when convenient (it transited chat); then update
  the 1P backup item + `hcloud context create ruxel-fixtures`.
- Optional: baseline production timing logs (OPERATOR-SETUP.md §3) — true
  "before" denominator; operator-run only, never autonomous.

---

## 7. Known workload findings (preserved, not bugs in ruxel)

- `config/sentry/config.yml` references `slack_client_id`/`_secret`/
  `_signing_secret` — defined nowhere in the workload; a real
  setup-sentry.yml run errors there (ruxel reproduces it; it's a golden).
- install-base's 10 `mise use -g …` tasks are bare `command` with no
  `changed_when` → always report changed under both ruxel and Ansible.

---

## 8. Commit trail this arc (newest first)

```
1411ba0 docs(bench): converged no-op — ruxel 1.04s vs ansible 15.16s (14.6x)
df20a35 feat(agent): convergence ledger — cached fingerprint fast-path
25d2b3a feat(oracle): dry-secrets bless harness for setup-* playbooks
d6eb598 feat(cli): --diff (copy/template unified diffs)
fc7a8ee feat(secrets): op-backed lookup resolver (real 1Password)
986aff2 feat: install-base.yml gated; bless-gate compares parity not zero
41cd091 feat(cli): --tags engine
63db936 feat(cli): --output json
fafcdc5 fix(postgresql): SQL via stdin (no password in argv) + grant allowlist
390eea9 feat: pause module — all 36 modules implemented
f1f40a5 feat(agent): PostgreSQL ×4 + become_user (both PG subtleties)
e02877d feat(agent): storage lvg/lvol/filesystem/mount on real volumes
17ed6da feat(agent): 11 modules (sysctl/lineinfile/replace/blockinfile/…/git/iptables/template)
bef4998 feat: get_url + apt_repository + copy src= — install-docker/upgrade-debian gates
```
plus the holla/holla-apt deployment fixes on those repos (PRs #9/#10/#11
on holla-apt; #21/#22/#23/#24 on holla — all merged).

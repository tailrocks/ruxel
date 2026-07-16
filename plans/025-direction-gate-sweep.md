# Plan 025: Sweep the available-now parity gates

> **Executor instructions**: Follow step by step; verify each. Honor STOP
> conditions. Update this plan's row in `plans/README.md` when done. This plan
> runs **against disposable fixtures only** — never a production host. Record
> `Safety check: target` (per `GOAL.md` rule 2) before any remote command,
> confirming the target IP is not one of the six production hosts.
>
> **Drift check (run first)**:
> `git diff --stat b5f98ba..HEAD -- tools/fixtures/ tools/oracle/`
> If the harness changed, re-verify the commands; on mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: M (per-gate S–M; mostly running an existing harness on fixtures)
- **Risk**: LOW (verification on disposable VMs; no code change expected)
- **Depends on**: none (but landing the correctness fixes 006/010/011/012 first
  means the gates verify the *fixed* code — sequence accordingly)
- **Category**: direction (verification breadth) — M4 coverage
- **Planned at**: commit `b5f98ba`, 2026-07-03

## Why this matters

Only **6 of 16** workload playbooks are gated three-way (ruxel apply → ruxel
rerun `changed=0` → real Ansible blesses the state). M4's bar is "every one of
the 16 playbooks converges on its fixture with end-state equivalence." The
**drive-variant** playbooks (`init-{titan,delorean,pegasus,nova,selene}-drives`)
and **restart-blockchain-nodes** need **no operator credential** — only fixture
VMs + volumes (operator pre-approved per `RESTORE.md:113-114`), and the storage
shape is already proven. Each gated playbook is one more piece of committed
evidence backing the "run ruxel instead of Ansible" claim, and it's the
highest-confidence, lowest-risk, zero-operator-dependency progress available.
(The six `setup-*` gates are **not** here — they're blocked on the operator
dropping a read-only ChainArgos deploy key, `RESTORE.md:84-93`; out of scope.)

## Current state

- Gated so far (`RESTORE.md:64-70`): `update-packages`, `upgrade-debian`,
  `install-docker`, drives (lvg/lvol/filesystem/mount), postgresql, `install-base`.
- Ungated, no-key-needed (`WORKLOAD.md:137-148`): `init-titan-drives`,
  `init-delorean-drives`, `init-pegasus-drives`, `init-postgresql-nova-drives`,
  `init-clickhouse-selene-drives` (two-tier), and `restart-blockchain-nodes`
  (36 identical `containerctl` shell restarts — always-execute by design, a good
  honest datapoint on whether ruxel's one-connection path helps the workload's
  worst duplication case, `WORKLOAD.md:174-177`).
- Harness: `tools/fixtures/bless-gate.sh <dest> <keyfile> <agent-bin> <playbook>
  "" [dry]` automates the three-way proof (`RESTORE.md:146-149`). Fixtures via
  `tools/fixtures/create.sh`; volumes via `hcloud volume create ... --label
  ruxel=fixture`; **always** `tools/fixtures/reap.sh` at session end.
- The drive-variants differ from the already-proven `drives` gate only in disk
  counts / VG names / filesystem (xfs vs ext4) — near-identical shape.

**Safety (absolute, `GOAL.md`/`AGENTS.md`)**: never contact the six production
hosts (pegasus, delorean, titan, sentry, postgresql-nova, clickhouse-selene, or
any IP in the real `hosts.ini`). Only targets are self-created `ruxel-fixtures`
VMs; verify each target IP ≠ all six production IPs before the first remote
command. Real secrets never enter fixtures/captures/commits (synthetic vault +
`--dry-secrets` only). At most 2 fixture VMs at a time; reap everything at the
end.

## Commands you will need

| Purpose | Command | Expected |
|---------|---------|----------|
| Create fixture | `RUXEL_FIXTURE_TYPE=cpx22 tools/fixtures/create.sh <suffix>` | VM up; prints SSH opts + `RUXEL_FIXTURE_KEY` |
| Add volume(s) | `hcloud volume create --name ruxel-fixture-vol-N --size 10 --server <name> --label ruxel=fixture` | volume attached at `/dev/disk/by-id/scsi-0HC_Volume_*` |
| Build agent (musl) | `mise exec -- cargo zigbuild --target x86_64-unknown-linux-musl -p ruxel-agent --release` | binary at `target/x86_64-unknown-linux-musl/release/ruxel-agent` |
| Gate a playbook | `tools/fixtures/bless-gate.sh root@<ip> <keyfile> <agent-bin> <playbook> ""` | three-way parity (ruxel rerun changed=0, ansible bless changed=0) |
| Reap | `tools/fixtures/destroy.sh <name>` then `tools/fixtures/reap.sh` | project empty |
| Confirm empty | `hcloud server list && hcloud volume list` | no lingering resources |

## Scope

**In scope**:
- Running `bless-gate.sh` against fresh fixtures for the 5 drive-variants +
  restart-blockchain-nodes; committing the resulting goldens to
  `tools/oracle/captures/`.
- Small harness fixes if a variant needs a different disk count/VG name (the
  gate is data-driven; prefer configuration over code).

**Out of scope**:
- The six `setup-*` gates (blocked on the operator deploy key).
- Any production contact.
- Fixing module bugs found during the sweep — if a gate fails because of a code
  bug (e.g. plan 012's lvol `+100%FREE` on a fresh disk), STOP and route it to
  the relevant plan; don't hand-patch here.
- Keeping fixtures/volumes overnight (operator OK required for kept paid
  resources — reap at session end).

## Git workflow

- Branch: `advisor/025-gate-sweep`
- Commit per gated playbook: `feat(gate): init-<host>-drives three-way parity`
  with the golden captures.
- Do NOT push/PR unless instructed.

## Steps

### Step 1: Precondition + safety check

Confirm the `hcloud` context is `ruxel-fixtures` and the project is empty
(`hcloud server list`, `hcloud volume list`). Before any remote command, record
`Safety check: target = <fixture-ip>, verified ≠ all six production IPs`. Build
the x86_64-musl agent.

**Verify**: `hcloud server-type list` succeeds via the context; the agent binary
exists.

### Step 2: Gate the drive-variants (one fixture + matching volumes each)

For each of `init-titan-drives` (2×NVMe→data→XFS), `init-delorean-drives`
(2×SATA→backup→ext4), `init-pegasus-drives` (7×NVMe→blockchain→XFS),
`init-postgresql-nova-drives` (3×NVMe→data→XFS), `init-clickhouse-selene-drives`
(two-tier data + clickhouse-hot):
1. Create a fixture; attach the matching **count** of volumes (`hcloud volume
   create ... --label ruxel=fixture`) to stand in for the NVMe/SATA disks.
2. Run `bless-gate.sh` for that playbook. The gate must show: ruxel fresh apply
   → ruxel rerun `changed=0` → Ansible bless `changed=0`.
3. Commit the golden capture. Destroy the fixture + volumes; reap.

Respect the 2-VM cap — do these serially, reaping between. The five differ only
in disk count / VG name / fs; if the gate needs a per-variant parameter, pass it
via the harness's existing knobs (not code).

**Verify** (per variant): `bless-gate.sh` exits success with `changed=0` on both
the ruxel rerun and the Ansible bless; `hcloud volume list` is empty after reap.

### Step 3: Gate restart-blockchain-nodes

Create a fixture. `restart-blockchain-nodes` is 36 identical
`eval "$(mise activate bash)" && ./containerctl.main.kts restart <node>` shell
tasks — these are **always-execute** by design (no `creates`/`changed_when`), so
"parity" here means the **changed-set matches Ansible** (both report all 36
changed every run), not `changed=0`. The `bless-gate.sh` "parity not zero" mode
(the same logic used for install-base's always-changed mise tasks,
`RESTORE.md:69`) applies. This gate also yields an honest datapoint: does ruxel's
one-connection native path meaningfully help 36 sequential shell restarts?
Record the wall-clock vs Ansible.

**Verify**: the changed-set for the 36 tasks is identical between ruxel and
Ansible; commit the golden. Reap.

### Step 4: Reap and update coverage

Destroy all fixtures + volumes; `tools/fixtures/reap.sh`; confirm
`hcloud server list && hcloud volume list` are empty. Update the coverage count
in `RESTORE.md`/`GOAL.md` (now 12/16 gated, or however many landed) — coordinate
with plan 002's count reconciliation.

**Verify**: `hcloud server list` and `hcloud volume list` → empty. The committed
captures for each gated playbook exist in `tools/oracle/captures/`.

## Test plan

This plan *is* verification — the "tests" are the three-way bless-gates on
fixtures. No unit tests. The evidence is the committed golden captures + the
`changed=0` (or parity-changed-set) gate output.

## Done criteria

ALL must hold:

- [ ] Synthetic storage fixtures cover the five extracted shapes (ext4, XFS, disk-count variation, VG variation, two-tier) with committed goldens
- [ ] `restart-blockchain-nodes` is gated (changed-set parity, not zero) with a committed golden + wall-clock datapoint
- [x] Every fixture + volume created is destroyed and reaped; `hcloud server list` and `hcloud volume list` are empty
- [x] No production host was ever contacted (`Safety check: target` recorded per remote session)
- [ ] Coverage count updated in `RESTORE.md`/`GOAL.md`
- [x] `plans/README.md` row for 025 updated

Synthetic replacement gates landed 2026-07-16 for ext4 storage, controller
delegation, and PostgreSQL schema ownership/default-privilege target roles.
Each passed Ruxel fresh apply, Ruxel converged rerun, and Ansible bless; all
resources were reaped. Remaining storage shapes and restart semantics stay
open.

2026-07-16 precondition attempt: active context was confirmed as
`ruxel-fixtures`, but both `hcloud server list` and `hcloud volume list` failed
to return. No fixture or volume was created, no SSH target was obtained, and no
remote host was contacted. Per Step 1 and the safety rules, the sweep did not
proceed.

Later the same day the CLI recovered. Runs of real workload playbooks against
fixtures were invalidated and their captures removed after the operator
clarified the fixture-project isolation rule. The ext4 resize bug they exposed
was reproduced with a synthetic minimal playbook and fixed structurally, but
all parity gates must use repository-owned synthetic playbooks going forward.

## STOP conditions

Stop and report if:
- A gate fails because of a **code** bug (e.g. lvol `+100%FREE` fails on a fresh
  disk — plan 012's CORRECTNESS-12) — STOP, route it to the owning plan, and
  re-run the gate after the fix. Do not hand-patch the module here.
- Any target IP matches a production host — **STOP immediately**, destroy
  fixtures, report (absolute safety rule).
- A fixture or volume can't be reaped — do **not** leave paid resources;
  escalate to the operator immediately with the resource IDs.
- More than 2 fixtures would be needed simultaneously — serialize instead
  (the cap is a safety/cost rule).

## Maintenance notes

- The six `setup-*` gates remain blocked on the operator's read-only ChainArgos
  deploy key — the natural next sweep once that lands (a separate plan).
- Reviewer: confirm the goldens were captured with `--dry-secrets`/synthetic
  values where any secret is involved, and that no real secret entered a fixture
  or capture.
- The `restart-blockchain-nodes` wall-clock is a useful honest number for the M5
  benchmark story (36 always-execute shells is the workload's worst case for a
  tool whose win is skipping converged work).

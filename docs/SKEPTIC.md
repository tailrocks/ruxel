# The Skeptic's Pass — Do We Need to Build This At All?

Status: analysis for operator decision. 2026-06-11. Read after
[DIRECTION.md](DIRECTION.md); this document deliberately attacks its
conclusion and the current Ansible usage, then rebuilds the recommendation
from evidence.

---

## 1. What this repository actually is (evidence, not assumption)

Git history of `ansible-configs` since 2026-01-01 (~5.3 months, 68 commits):

| File | Commits | Share | What the changes are |
|---|---|---|---|
| `setup-sentry.yml` | 30 | 44% | CI-host iteration: Velnor runners, GARM removal, runner slots, sccache, CI image fixes, inotify limits |
| `setup-delorean.yml` + `config/` | 23 | 34% | App deployment: new env files, nginx/platform deploys, readiness configs, Docker Hub setup |
| All other 14 playbooks | 15 | 22% | Genuine server config, mostly single-digit touches |

Two conclusions that change the whole analysis:

1. **This is not a provisioning repo. It is a daily-driver deployment system.**
   ~68 commits ≈ 100+ playbook reruns in five months, several per week, each
   paying the full multi-minute cost. The dominant loop is *change one task →
   rerun a 50-task playbook → wait minutes for one change to land*. The pure
   "converged no-op" is the second-most-common case, not the most common.
2. **78% of the churn is not server configuration.** It is application/CI
   deployment (env files, runner daemon config, container restarts) riding
   inside server-provisioning playbooks. That work changes daily and pays a
   provisioning-pipeline price every time.

So the honest re-statement of the problem: **three different jobs are bundled
into one slow tool** —

- **Job A — server provisioning/config** (drive init, PostgreSQL/ClickHouse
  install, kernel tuning): changes rarely; correct tool category is config
  management; Ansible is *shaped* right and merely slow.
- **Job B — app/CI deployment** (18 env files, Velnor config, restarts):
  changes constantly; this is continuous deployment, and no config-management
  engine — however fast — is the right primary tool for it.
- **Job C — state verification** ("prove the server is exactly right, find
  what drifted"): wanted casually and often; **no tool on the market does
  this fast against in-OS reality.** This is the genuine gap.

## 2. Finding: the current Ansible is completely untuned

`ansible --version` on the controller: **`config file = None`** — every
performance-relevant setting is at its default: no pipelining, default
`forks=5`, full fact gathering each play, no fact caching, no callback
profiling, `ControlPersist=60s`. The 1Password lookups are re-evaluated on
every template reference (13 declared lookups in `setup-postgresql-nova.yml`
alone, multiplied by every task/loop iteration that references them).

Two corrections to DIRECTION.md's interim-relief idea:

- **Mitogen is OFF the table**: the controller runs ansible-core 2.21;
  Mitogen supports 2.10–2.14 only.
- Everything else is available *today* with a 20-line `ansible.cfg` plus a
  small playbook refactor (freeze each lookup once via `set_fact`):
  pipelining, `gathering = smart` + JSON fact cache, longer ControlPersist,
  `profile_tasks` callback for real per-task timings. Public benchmarks put
  this class of tuning at a 50–70% wall-clock cut. Expected: **15 min →
  roughly 3–5 min**, for an afternoon of work and no new tool.

This matters for honesty: part of the 15-minute pain is self-inflicted
defaults, not Ansible's ceiling. The *ceiling* argument stands — tuned
Ansible still cannot reach seconds, because nothing in it remembers prior
verification — but the baseline for any build-vs-buy decision should be the
tuned number, measured with `profile_tasks`, not the untuned one.

## 3. Is OpenTofu the answer? (asked directly)

No, for in-OS state — and the reason is precise: Tofu trusts its **state
file**; the operator's core requirement is distrust of the server ("someone
made a dirty change → find it and fix only it"). Reality-probing is exactly
the part Tofu does not do for OS internals (no maintained provider reads
dpkg/systemd/LVM/pg_catalog over SSH). Building that provider means building
ruxel's engine anyway, then living inside HCL + the provider gRPC protocol,
and rewriting all 16 playbooks. Tofu remains right for the layer *above*
(ordering Hetzner servers, DNS, cloud firewall) and for provisioning ruxel's
disposable test VMs — optional, additive, not a replacement.

## 4. Is NixOS the answer? (the strongest "wrong tool" candidate)

If the servers were NixOS machines, Jobs A and C dissolve by construction:
the whole OS converges from a declaration, drift in managed config is
structurally impossible, and "verify" is `nixos-rebuild switch` against an
unchanged config. This is the most credible "you are using the wrong tool"
argument, so it was researched in depth (2026-06-11, all claims sourced):

**For it:**
- Hetzner dedicated installs are a solved path: `nixos-anywhere` (kexec from
  rescue) + `disko` declarative LVM/XFS — both actively maintained.
- The stack is mostly packageable: PostgreSQL 18 is in nixpkgs 25.11;
  `buildPgrxExtension` exists and nixpkgs pins **cargo-pgrx 0.16.1 — the
  exact version pg_parquet 0.5.1 requires**; packaging pg_parquet is ~30
  lines by the in-tree pattern. ClickHouse module supports native
  `config.d`/`users.d` layering incl. tiered storage XML. Docker +
  `oci-containers` handle the ~50 containers. `opnix` (active, 2026)
  integrates 1Password via service-account token at activation.

**Against it (decisive for this fleet, today):**
- **The headline metric fails.** Real-world no-op `nixos-rebuild switch` is
  **10–20 seconds locally** (eval cost grows with config size) — before
  remote-deploy overhead. NixOS does not beat the ruxel target; it roughly
  ties tuned-Ansible's *per-change* latency and loses to a fingerprint
  ledger on verification.
- **The hottest workload is NixOS-hostile.** `setup-sentry.yml` is 44% of
  all churn, and Sentry upstream explicitly **refuses to support NixOS**
  (getsentry/self-hosted #2160: "we do not plan to support NixOS"); the only
  community attempt is abandoned at "not fully working". The Velnor/CI
  iteration loop would still be imperative work around a compose installer.
- **Migration risk is exactly the forbidden kind.** `nixos-anywhere` wipes
  the disks it manages; preserving data LVs is possible (`--disko-mode
  mount`, scoping disko to the OS disk) but crossing glibc builds under a
  live PostgreSQL **silently corrupts collation-dependent indexes** (REINDEX
  required — on TB-scale clusters and a 28 TiB ClickHouse volume). In-place
  conversion (`NIXOS_LUSTRATE`) is deprecated for removal. These are
  production database servers the operator has ruled untouchable.
- Team cost: one operator, deep Debian fluency, six pet servers; the
  documented NixOS failure-mode genre (under-documented modules, escape
  hatches, "an additional layer when things go wrong") prices in poorly at
  this scale.

**Verdict:** NixOS is the right structural answer for a *greenfield,
NixOS-friendly* fleet, and remains a live option for **future non-database
servers** if one is ever ordered. It is the wrong migration for these six
machines now, and even when it fits, its no-op latency does not deliver the
seconds-level verification this project is for.

## 5. Re-derived recommendation (the ladder)

Each step stands alone, has its own kill-criterion, and never touches
production without the operator running it.

**Step 0 — Measure and tune what exists (this week, no new tool).**
`ansible.cfg` (pipelining, smart gathering + fact cache, ControlPersist,
forks, `profile_tasks`) + freeze every 1P lookup via `set_fact` once per
play. The operator runs one timed rerun per key playbook. Deliverables: real
per-task timings and the honest baseline. *Kill criterion: if 3–5 min is
livable, stop here — no project needed.* (The operator has said seconds
matter; this step still pays for itself by making every later comparison
honest and by giving immediate relief.)

**Step 1 — Unbundle Job B (structural, cheap, tool-agnostic).** Move the
daily-churn deploys (env files, Velnor config + restart, container
restarts) out of the 50-task playbooks into a thin fast path — whether
that's a 100-line script, a `just` target, or ruxel's first subcommand. A
one-file change should cost seconds *by construction*, regardless of
executor. This removes ~78% of rerun occasions from the slow path without
touching the provisioning logic.

**Step 2 — Build ruxel, verify-engine first.** The market gap (Job C) is
real: fast reality-probing verification with drift pinpointing does not
exist. But stage it to kill risk early:

- **2a. `ruxel plan` only** — parser + renderer + remote probes + ledger,
  **read-only by design**: answers "is this server exactly per playbook,
  what drifted" in seconds. Ansible remains the apply engine (run with
  `--limit`+the drifted playbook when `ruxel plan` finds work). Read-only
  cuts the hardest risk class (apply-semantics parity) out of the first
  deliverable entirely, and is independently valuable forever.
- **2b. `ruxel apply`** — the full executor from DIRECTION.md — only after
  2a proves the fidelity layer (parsing, templating, probe correctness) on
  real workload files against disposable VMs. *Kill criterion: if 2a +
  tuned-Ansible-apply already feels instant in practice, 2b may never be
  needed.*

**Ambitious options on top (operator's appetite):**
- **Pull-mode daemon / GitOps**: the 2a verify engine running as a tiny
  daemon per host, continuously probing and reporting state to a dashboard
  or Slack — "is everything correct?" becomes a glance, zero seconds, and
  drift is reported before anyone asks. Same binary, same ledger; natural
  v3.
- **Tofu for the outer layer + test fixtures**; **NixOS for future
  greenfield non-DB hosts**; both optional and orthogonal.

## 6. Bottom line

- The usage is not "wrong Ansible" — it is **three jobs in one tool**, one of
  which (daily app/CI deploys) should leave the slow path regardless of
  executor, and one of which (instant verification) no existing tool serves.
- The 15-minute number is partly untuned defaults; the honest competitor for
  ruxel is **tuned Ansible at ~3–5 min**, which still cannot reach seconds.
- OpenTofu: wrong layer. NixOS: right idea, fails this fleet's constraints
  *and* the latency target. Buying is not an option for Job C.
- Therefore: tune first (Step 0), unbundle deploys (Step 1), and build ruxel
  **verify-first** (Step 2a) — the smallest artifact that delivers the
  seconds-level guarantee — growing into the full executor (2b) only if
  reality still demands it.

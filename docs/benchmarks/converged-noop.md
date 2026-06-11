# Benchmark — converged no-op (the "seconds on converged hosts" claim)

Wall-clock of a re-run against an already-converged host: the case the
operator hits constantly (nothing to change). Measured 2026-06-12 on a
Hetzner cpx22 x86_64 Debian 12 fixture in `ruxel-fixtures` (best of 3,
warm), playbook `install-docker.yml` (8 tasks: apt cache/install,
apt_repository, get_url, file, copy+handler, service started+enabled).

| Run | Wall-clock | vs Ansible |
|-----|-----------:|-----------:|
| **ruxel apply (ledger cached)**      | **1.04 s** | **14.6× faster** |
| ruxel apply (`--no-cache`, full native checks) | 4.04 s | 3.8× faster |
| ansible-playbook (converged no-op)   | 15.16 s | 1.0× (baseline) |

Reading:
- **Ledger path (1.04 s):** ~0.5 s is SSH ControlMaster connect + agent
  handshake; the actual convergence verdict over the task set is sub-second
  (cached fingerprints re-verify in parallel, modules not invoked).
- **`--no-cache` (4.04 s):** full native checks every task — still 3.8×
  faster than Ansible with zero caching, because there is no per-task SSH
  round-trip, no remote Python, and dpkg/systemd state is read in batched
  snapshots rather than re-interrogated per task.
- **Ansible (15.16 s):** per-task SSH exec + AnsiballZ Python payload upload
  per module is the tax ruxel removes.

Method: `time -p` best-of-3, same converged fixture state (ruxel applied
it, `ansible-playbook` blessed it changed=0 first). Reproduce with the
release binaries against any fixture:
`ruxel apply -i <inv> install-docker.yml` (cached) vs `--no-cache`, and
`ansible-playbook -i <inv> install-docker.yml` under the pinned 2.21 venv.

Next: per-playbook profile_tasks breakdown and the 65-task
`setup-postgresql-nova` converged number (pending the setup-* fixture +
private-repo deploy key — GOAL.md), plus a fresh-provision and
6-hosts-parallel run for the full M5 report.

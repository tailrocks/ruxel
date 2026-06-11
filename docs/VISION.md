# Ruxel — Vision

> Rust-native automation without the YAML archaeology.

## The problem

ChainArgos provisions and maintains 6 dedicated Debian servers with Ansible
(16 playbooks, 452 tasks, 29 distinct modules). The playbooks work, but every
run is slow — and the worst case is the most common one: **re-running a
playbook against a server that is already fully converged takes ~15 minutes to
conclude "nothing to do".**

That cost has consequences:

- Re-running pipelines to *prove* a server is in the correct state is so
  expensive that it is avoided.
- The workaround — `--tags` / `--limit` selection — undermines the whole point:
  a partial run proves nothing about the state of the server.
- Spawning a new server means waiting through the full sequential cost, even
  for the parts that parallelize trivially.

The root causes are structural to Ansible, not fixable with configuration:
per-task Python payload upload + interpreter spawn on the remote host,
sequential task execution, sequential secret lookups (each 1Password lookup is
its own `op` subprocess), and re-verification of every task from scratch on
every run.

## The vision

Ruxel is a **drop-in executor for exactly the Ansible workload ChainArgos
has** — same playbook files, same `hosts.ini`, same `--check`/`--diff`/
`--limit` invocation shape — rebuilt around one goal:

**Never repeat work that is already done, and prove it in seconds.**

```
ruxel plan  -i hosts.ini --limit postgresql-nova setup-postgresql-nova.yml   # seconds: full diff
ruxel apply -i hosts.ini --limit postgresql-nova setup-postgresql-nova.yml  # applies only the diff
```

A converged server answers "0 changed, verified" in seconds. A drifted server
(someone made a manual change) is detected by cheap fingerprint probes, and
only the drifted tasks are re-applied. A fresh server runs the full plan with
maximum parallelism. The mental model is Terraform's plan/apply — but for
in-OS state, with drift detection done *on the server* by a native agent
instead of trusted from a state file.

## Principles

1. **Performance is the feature.** Every design decision is judged first by
   its effect on wall-clock time, with the converged no-op rerun as the
   headline benchmark.
2. **Everything is Rust. Everything.** The controller is Rust. The thing that
   executes on the server is a single static Rust binary delivered over SSH.
   No Python on the targets, ever. No interpreter startup, no module upload
   per task.
3. **Closed scope, total fidelity.** Ruxel implements exactly the module set
   and language features the ChainArgos playbooks use ([WORKLOAD.md](WORKLOAD.md)
   is the closed spec) — and implements them with exact semantics. It is not a
   general Ansible replacement and does not accept features outside the spec.
4. **A full rerun must always be cheap.** No selection mechanisms as a
   performance crutch. The way to know a server is correct is to run the whole
   pipeline, and that has to be fast enough to do casually.
5. **Verification is a first-class operation.** "Is this work already done?"
   has a fast path (fingerprint cache + cheap probes) and a correct path
   (full module-level check), and the fast path must never sacrifice
   correctness — any doubt falls through to the real check.
6. **Parallel by default.** Hosts in parallel, independent tasks in parallel,
   secret lookups in parallel, verification probes in parallel. Sequential
   only where ordering is semantically required.

## Goals

- Parse and execute the existing ChainArgos playbooks, inventory, templates,
  and 1Password lookups **without changing a single file**.
- Converged no-op rerun of the largest playbook (`setup-postgresql-nova.yml`,
  65 tasks): **under 5 seconds** end-to-end, against ~15 minutes today.
- First-run provisioning time dominated by actual work (apt downloads, LVM
  operations), not by executor overhead.
- Accurate `plan` (check + diff) output that can be trusted as a statement of
  server state.
- Drift introduced by manual changes is detected and corrected by re-running
  the same pipeline, touching only what drifted.

## Non-goals

- General-purpose Ansible compatibility, Galaxy, roles, collections as a
  plugin ecosystem, or any module/feature not present in the ChainArgos
  workload.
- Multi-tenancy, RBAC, web UI, or anything aimed at "everyone". This tool is
  for this workload.
- Replacing the infrastructure-creation layer (Hetzner server ordering).
  Ruxel manages state *inside* servers; it borrows Terraform's UX, not its job.

## Hard safety rule

**During research, design, and development, ruxel must never connect to,
probe, or execute anything against the production servers.** All six hosts in
`hosts.ini` are production. Development and benchmarking happen exclusively
against disposable targets (local VMs / containers / throwaway cloud hosts)
that the operator provides explicitly. This rule has no exceptions and no
expiry until the operator lifts it per-occasion.

## Related documents

- [WORKLOAD.md](WORKLOAD.md) — the closed compatibility spec: every module,
  feature, and pattern the ChainArgos playbooks use, with counts.
- [DIRECTION.md](DIRECTION.md) — problem analysis, prior art, recommended
  architecture, and alternatives considered.

# Synthetic PR 66 workloads

These fixtures are ports of individual workloads in the frozen 99fb2d17
fixture matrix. Their loopback inventory is a placeholder: a harness must
replace it with its isolated disposable hosts before any remote execution.
The harness must replace `__RUN_ROOT__` in evaluated configurations and
expectations with one isolated temporary directory for each run.
Replace `__FIXTURE_ROOT__` with the absolute local path of this directory
when evaluating source assets; never resolve it as a remote host path.
The files only describe workloads; they do not create hosts or run commands.

`expectations.json` specifies file observations, a drift injection and phase
outcomes for the cases implemented here. The harness must compare independent
snapshots before and after check and diff; successful process exit alone does
not establish nonmutation. Task-result observations must be gathered independently by the harness.
`changed` refers to the copy task,
not necessarily the entire play (pause and handlers have separate semantics).

Pkl evaluation has been checked against the in-development `schema/Ruxel.pkl`.
All fourteen fixtures passed the integrated planner with synthetic secret
resolution; planning does not execute or validate the operations. The
check-semantics fixture passed local plan, check, diff, fresh apply,
converged apply and drift repair against the in-development controller. The
other workloads and their state expectations remain incompletely verified;
this directory does not establish full PR 66 parity.

Storage scenarios require isolated disposable fixture disks at their declared
by-id paths. Their drift-repair coverage remains pending. Never execute them
on a real server.

`shape-variants.pkl` omits two frozen tasks that were permanently skipped but
provide conflicting operation fields. Their original shapes are retained in
`invalid/` as schema rejection cases. Both are rejected by the current schema.

`system-surface.pkl` changes root authorized keys, firewall, mounts, services,
accounts and host configuration. It is permitted only on dedicated disposable
fixture VMs, never on local or real servers. Its forced check-mode HTTP server
may change PID and log files; the nonmutation requirement is not yet proven.

Storage drift specifications erase filesystem signatures. A harness must prove
the exact target is a disposable fixture volume before applying them; no drift
injection has been executed. Snapshot call-count expectations require separate
instrumentation and have not been measured.

The delegation fixture forces its read-only local `printf` to run in check
and diff so its following assertion has a result. The frozen task did not
force check mode; this deliberate difference needs separate parity coverage.

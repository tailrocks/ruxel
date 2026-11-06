# Synthetic PR 66 workloads

These fixtures are ports of individual workloads in the frozen 99fb2d17
fixture matrix. Their loopback inventory is a placeholder: a harness must
replace it with its isolated disposable hosts before any remote execution.
The harness must replace `__RUN_ROOT__` in evaluated configurations and
expectations with one isolated temporary directory for each run.
The files only describe workloads; they do not create hosts or run commands.

`expectations.json` specifies file observations, a drift injection and phase
outcomes for the cases implemented here. The harness must compare independent
snapshots before and after check and diff; successful process exit alone does
not establish nonmutation. Task-result observations must be gathered independently by the harness. `changed` refers to the copy task,
not necessarily the entire play (pause and handlers have separate semantics).

Pkl evaluation has been checked against the in-development `schema/Ruxel.pkl`.
These fixtures have not been run through the controller or an acceptance
harness. The other seven matrix workloads and their state expectations remain
unported; in particular this directory does not establish full PR 66 parity.

Storage scenarios require isolated disposable fixture disks at their declared
by-id paths. Their drift-repair coverage remains pending. Never execute them
on a real server.

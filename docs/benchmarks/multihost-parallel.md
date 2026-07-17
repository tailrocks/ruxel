# Two-host parallel execution

> Historical auxiliary measurement. Superseded by the current repeated
> [six-host evidence](results/six-host/) and [acceptance matrix](README.md).

Measured 2026-07-16 on two provider-labeled disposable Debian 12 fixtures in
the isolated fixture project. Source was the repository-owned
`tools/fixture-project/files-content.yml`; no production or reference-workload
target was contacted.

The same converged target state and controller binary were used for three
wall-clock samples:

| Selection | Wall time |
|---|---:|
| fixture A only | 1.31 s |
| fixture B only | 1.54 s |
| A + B together | 1.42 s |

The combined run completed in approximately the slower single-host time, not
their 2.85 s sum. Task output was buffered and emitted in inventory order;
both hosts produced complete independent recaps. Sequential and concurrent
connections in one controller process completed without the historical
second-connect stall. Both fixtures and ephemeral keys were reaped after the
measurement.

This proves the two-host orchestration gate. It does not replace the separate
six-host benchmark required by the final performance matrix.

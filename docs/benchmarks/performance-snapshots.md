# Secret and system snapshot benchmark

Captured 2026-07-16 on a disposable x86_64 Debian 12 Hetzner fixture in the
isolated `ruxel-fixtures` project. The synthetic playbook exercises two fields
from one dry-secret item, apt package checks, systemd state, and two PostgreSQL
role checks sharing one session. No real workload file or secret was executed.

| Executor | Converged wall time | Changed |
|---|---:|---:|
| Ruxel | 4.41 s | 0 |
| Ansible core 2.21.2 | 24.67 s | 0 |

Ruxel was **5.59× faster** with identical converged status. Commands were timed
individually using `/usr/bin/time -p` against the same fixture. Oracle evidence:
`tools/oracle/captures/bless-performance-snapshots.jsonl`.

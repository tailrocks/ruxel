# Repeated restart-action benchmark

> Historical one-shot auxiliary measurement, not acceptance-grade performance
> evidence. See the repeated [acceptance matrix](README.md).

Captured 2026-07-16 on a disposable x86_64 Debian 12 Hetzner fixture in the
isolated `ruxel-fixtures` project. The repository-owned synthetic playbook
`tools/fixture-project/restart-actions.yml` executes 36 always-changed shell
actions. No real workload file, inventory, hostname, secret, or target was
used.

| Executor | Converged wall time | Changed result |
|---|---:|---|
| Ruxel | 3.12 s | 1 loop task / 36 changed items |
| Ansible core 2.21.2 | 173.34 s | 1 loop task / 36 changed items |

Ruxel was **55.6× faster** while matching Ansible's changed-set. Commands were
timed individually with `/usr/bin/time -p` against the same converged fixture.
The committed oracle capture is
`tools/oracle/captures/bless-restart-actions.jsonl`.

# Six-host disposable-container benchmark

> Historical auxiliary measurement. Superseded by the current repeated
> [six-host evidence](results/six-host/) and [acceptance matrix](README.md).

Measured 2026-07-16 with `tools/fixtures/local-six-host-gate.sh` against six
fresh Debian 12 SSH containers labeled `ruxel=local-fixture`. The synthetic
workload is `tools/fixture-project/multihost.yml`: one independent one-second
command per host. No production inventory, address, or configuration is read.

Environment: Docker 29.4.0 on Apple Silicon; both executors contact x86-64
Debian containers over the local Docker bridge. Ruxel uses the committed
x86-64 static agent and a warm content-addressed upload cache. Ansible uses
the pinned `tools/oracle` environment with six forks.

| Executor | Six-host wall time |
|---|---:|
| Ruxel | 2.50 s |
| Ansible 2.21 | 5.97 s |

Ruxel's six-host run was close to the slowest isolated host (1.47 s), and far
below the measured sequential sum (8.67 s). The accepted sample exercised nine
Ruxel runs, required six recaps in inventory order, then inspected every
container. No SSH child process remained. It then stopped the sixth target and
proved five ordered recaps, one structured unreachable record, and failure exit
status. The gate destroys all six containers and the ephemeral SSH key on exit.

Reproduce:

```sh
cargo zigbuild --target x86_64-unknown-linux-musl --release -p ruxel-agent
tools/fixtures/local-six-host-gate.sh \
  target/x86_64-unknown-linux-musl/release/ruxel-agent 10
```

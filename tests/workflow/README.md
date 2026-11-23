# Workflow language corpus

Workflows written in the language that `modulepath:/ruxel/Workflow.pkl`
serves: `Core.pkl`, which it extends, and the layer modules beside it
(`Linux.pkl`, `Systemd.pkl`, `Debian.pkl`).
`crates/ruxel-config/tests/workflow.rs` evaluates every file:

- `valid/`: each workflow must evaluate, pass admission and produce plays.
- `invalid/`: each workflow must fail, and the error message must contain the
  text of its `// expect: <text>` line. Where pklr words the error differently
  from Pkl, `// expect-pklr: <text>` gives pklr's text, which ruxel uses.

`inventory.pkl` uses documentation addresses only (RFC 5737). `recipes/` holds
two example recipes that several cases share. `resources/` holds the files
a workflow reads with `read("…")`: no import reaches them, so a walk that
follows imports alone would never name them.

Every workflow sets the required `name`: the stable identity the engine
namespaces its ledger by, so two workflows that reuse play and host names
never share ledger state. Fixtures use their corpus path, which is unique
here for the same reason; name a real workflow after what it does.

## Timer workloads

A timer workload is the whole composition, in order: the executable the
service runs (a `File` with mode `0755`), the `.service` unit file, the
daemon-reload `Command` after the unit file, and the `.timer` step — which
owns its unit file and its enablement. See
`valid/systemd-timer.pkl`: the multixact monitor fires at boot (`onBoot`)
and every five minutes after its last firing (`repeat`). The steps run as
the SSH user, and the inventory connects as root (`Host.user` defaults to
`"root"`), so no step of a timer workload names another account: there is
no `becomeUser` on a root-owned unit file or a `systemctl` run.

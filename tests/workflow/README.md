# Workflow language corpus

Workflows written in the language that `modulepath:/ruxel/Workflow.pkl`
serves: `Core.pkl`, which it extends, and the layer modules beside it
(`Linux.pkl`, `Systemd.pkl`, `Debian.pkl`).
`crates/ruxel/ruxel-config/tests/workflow.rs` evaluates every file:

- `valid/`: each workflow must evaluate, pass admission and produce plays.
- `invalid/`: each workflow must fail, and the error message must contain the
  text of its `// expect: <text>` line. Where pklr words the error differently
  from Pkl, `// expect-pklr: <text>` gives pklr's text, which ruxel uses.

`inventory.pkl` uses documentation addresses only (RFC 5737). `recipes/` holds
two example recipes that several cases share.

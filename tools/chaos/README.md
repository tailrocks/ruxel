# Chaos gate

Local protocol chaos coverage lives in
`crates/ruxel-agent/tests/protocol.rs`:

- EOF immediately after `HelloAck`;
- truncated frame during the next `Plan`;
- controller disconnect after `TaskStart`, followed by ledger flush, lock
  reacquisition, and a converged rerun;
- existing abrupt `kill -9`, malformed-frame, and single-run-lock cases.

Run locally:

```sh
cargo nextest run -p ruxel-agent chaos
```

## Fixture-only states

Real network failure behavior still needs an operator-provided disposable VM.
Never use production targets. For each fixture, record `Safety check: <target>`
before contact, then inject SSH transport loss at these boundaries:

1. while the controller uploads/starts the agent;
2. after `Hello` but before the complete `HelloAck` reaches the controller;
3. while a large `Plan` or `TaskResult` crosses the SSH channel;
4. during a long-running non-atomic module subprocess;
5. during controller Ctrl-C with an established ControlMaster.

After every injection, reconnect in a new process, prove the flock is free,
rerun the same play, and require converged `changed=0` for fingerprintable
tasks. Keep fixture IPs and timing evidence in the gate report; never resolve,
probe, or contact hosts listed in `AGENTS.md`.

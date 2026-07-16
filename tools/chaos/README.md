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

## Acceptance artifact

The future remote gate must write `tools/chaos/artifacts/manifest.json`, then
run:

```sh
python3 tools/chaos/verify.py
```

Run the executable gate only with a provider identity returned by the isolated
fixture project (never an address):

```sh
tools/chaos/gate.sh \
  ruxel-fixture-<session> /tmp/ruxel-fixture-<session>-ssh \
  target/release/ruxel target/x86_64-unknown-linux-musl/release/ruxel-agent
```

The normal entry point owns and always reaps the disposable server and key:

```sh
tools/chaos/run.sh \
  target/release/ruxel target/x86_64-unknown-linux-musl/release/ruxel-agent
```

The gate provider-verifies that identity before its first SSH command. Its
PATH-scoped `ssh` shim delegates to the real OpenSSH binary and cuts exact
varint-framed protocol messages only for the selected synthetic fault. Each
recovery uses a new controller process. Direct `gate.sh` callers own their
fixture; `run.sh` is preferred because its exit trap destroys the labeled
fixture on success, failure, and interruption.

`make_payload.py` materializes the ignored deterministic 2 MiB copy source
immediately before the run; the gate trap removes it. This makes the large
`Plan` boundary real without committing a generated binary-sized fixture.

The committed manifest is normalized: `target` is exactly `<fixture>` and it
contains no IP addresses, secrets, controller paths, raw logs, or arbitrary
extra fields. Runtime-only logs may retain fixture connection details outside
the committed artifact.

Schema version 1 requires six independently injected boundaries. Splitting
the two wire directions prevents a partial large `Plan` from being mistaken
for evidence about a completed module's large `TaskResult`:

- `upload-start`
- `partial-hello-ack`
- `large-plan`
- `large-task-result`
- `long-subprocess`
- `controlmaster-sigint`

Each case has exactly these fields:

```json
{
  "case": "partial-hello-ack",
  "injection_sentinel": true,
  "interrupted_status": 130,
  "reconnect": true,
  "flock_free": true,
  "converged": true,
  "converged_changed": 0,
  "converged_failed": 0,
  "state_equal": true,
  "no_process_leak": true,
  "no_socket_leak": true,
  "no_temp_leak": true,
  "recovery_elapsed_ms": 250,
  "recovery_timeout_ms": 30000
}
```

`injection_sentinel` proves the requested boundary was observed before the
controller was interrupted. `interrupted_status` must be nonzero. Recovery
must finish inside its declared positive timeout, capped at 120 seconds. A
reconnected run must acquire the agent flock, report zero changed and failed
tasks, reproduce the seeded converged state, and leave no agent/module
process, ControlMaster socket/process, or interrupted-upload temporary file.

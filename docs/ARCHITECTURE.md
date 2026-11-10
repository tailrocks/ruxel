# Architecture

Ruxel evaluates typed Pkl into a configuration, validates it, and runs tasks
through a Rust controller and agent. The implementation is under active
migration; this document describes the current interfaces, not a claim of
complete platform coverage.

## Components

- `crates/ruxel/ruxel` provides the command-line interface and exit codes.
- `crates/ruxel/ruxel-config` evaluates Pkl using pklr. The evaluator receives
  no inherited environment variables or external properties and no network.
  Module imports and `file:` resources are confined to the root directory
  (`--root-dir`, default the root module's directory) and the `--module-path`
  directories: each is opened once, and every path component is opened
  relative to its parent descriptor without following symlinks, so `..`,
  outside absolute paths and symlinks (even ones swapped in during evaluation)
  are refused, as are `import*` and `read*` globs. Templates are confined the
  same way to their source directory. Its module path serves the compiled-in
  modules under `modulepath:/ruxel/`, followed by the `--module-path`
  directories. Evaluation runs on its own thread with a 64 MiB stack and a
  pklr depth limit of 256.
- `schema/ruxel/` holds every served module: the workflow language
  (`Workflow.pkl`, `Ruxel.pkl`, `internal/Lowering.pkl`) and the engine schema
  (`engine/Engine.pkl`), which declares inventory, task, operation,
  expression, and lookup types. They are compiled into the binary, which
  replaces the `Config.schemaFingerprint` placeholder in `Engine.pkl` with
  `blake3:<hash of the module tree>@<nonce>`;
  the nonce is drawn once per process, so no file copy can carry it.
  `ruxel schema export` writes the hash alone. Before execution, `ruxel-config`
  admits only a configuration that carries this process's fingerprint and
  removes the property; the error echoes a fingerprint only if it is well formed. `ruxel-engine` then validates
  the evaluated value as well.
- `crates/ruxel/ruxel-engine` contains the model, controller, expression and
  secret resolution, runtime, and SSH transport.
- `crates/ruxel/ruxel-proto` defines messages shared by controller and agent.
- `crates/ruxel/ruxel-agent` executes host operations and returns results.

## Execution

1. The CLI evaluates the root module. `config dump` prints that value; execution
   commands also validate it against the model.
2. The controller selects inventory hosts and tasks. It evaluates runtime
   expressions against host variables, facts, loop items, and registered results.
   Lookup references are resolved by the controller rather than embedded as
   literal credentials in configuration.
3. The runtime connects to selected hosts using SSH and runs an agent. It passes
   operation requests and receives results through the protocol. A local agent
   mode exists for disposable local testing.
4. The controller handles task outcomes, conditions, retries, blocks and
   handlers, and reports per-host results. Plan selects operations without
   connecting, but censors task and host details because evaluated Pkl values
   do not carry file provenance. Check and diff use
   non-mutating checks where the operation supports them; a task with
   `checkMode = false` can execute changes in either mode. Apply performs changes.

Controller and agent use protocol version 3 and reject older peers during
handshake. The agent reports its durable sensitivity state in the handshake;
an omitted state is an error. See [Secret provenance](secret-provenance.md)
for the marker's scope and output consequences. `Copy.src` is read as bytes on
the controller and sent in an optional binary iteration payload; an empty
source differs from an absent source. Text
`Copy.content` and rendered native templates retain their text representation.
Source bytes are accepted only for Copy and cannot accompany explicit content.
Binary diffs report sizes and SHA-256 hashes; `noLog` suppresses those results.
The entire encoded request remains limited to 64 MiB, including all iterations
and parameters. A streaming file channel is not implemented, so larger copies
remain unsupported by this transport.

The CLI supports JSON and human-readable output. Optional JSONL event logs are
created as new private files. Treat generated output as sensitive: configuration
dumps can contain values supplied directly in Pkl, and secret redaction must be
verified for the workflow in use.

## Scope and safety

Schema definitions describe the accepted configuration surface; implementation
and regression evidence must be checked separately. The repository's local
acceptance suite exercises synthetic scenarios, but passing a subset does not
prove all providers or remote platforms are complete.

SSH uses strict host-key checking by default. Before any remote connection or
agent upload, the controller runs `ssh -G` with the connection's options and
destination (no SSH connection; OpenSSH still runs any `Match exec` commands in
the operator's configuration, which the deny list does not govern), requires the resolved `hostname` to be a
literal IP address with no proxy hop, and refuses it if it matches a deny entry
compiled from `RUXEL_GUARD_DENY_FILE`. Every later SSH and SFTP invocation for
that connection pins the admitted address with `HostName` (`%` escaped) and
disables `ProxyJump`, `ProxyCommand` and connection multiplexing, so
configuration changes, `Match exec` or an existing master cannot redirect it.
All invocations set `CanonicalizeHostname=no`, `ClearAllForwardings=yes`,
`PermitLocalCommand=no`, `KnownHostsCommand=none`, `VerifyHostKeyDNS=no` and
`GSSAPIAuthentication=no`, and disable agent, X11 and tunnel forwarding; the probe runs in its own process group, bounded by a
timeout that kills the group, and unspecified addresses (`0.0.0.0`, `::`) are
refused. `ruxel --version` reports how many
entries were compiled. This is a deny list only: the invoking environment is
still responsible for target admission and its own allow policy. Check and diff connect and inspect remote state. Only operate on explicitly authorized targets, and use isolated fixtures
for development. Never commit real credentials, host identities, or sensitive
captures to this public repository.

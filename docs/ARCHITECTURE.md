# Architecture

Ruxel evaluates typed Pkl into a configuration, validates it, and runs tasks
through a Rust controller and agent. The implementation is under active
migration; this document describes the current interfaces, not a claim of
complete platform coverage.

## Components

- `crates/ruxel/ruxel` provides the command-line interface and exit codes.
- `crates/ruxel/ruxel-config` evaluates Pkl using pklr. The evaluator receives
  no inherited environment variables or external properties and permits local
  file resources.
- `schema/Ruxel.pkl` declares inventory, task, operation, expression, and
  lookup types. `ruxel-engine` validates the evaluated value as well, including
  configurations that do not use the schema.
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
destination (no network contact), requires the resolved `hostname` to be a
literal IP address with no proxy hop, and refuses it if it matches a deny entry
compiled from `RUXEL_GUARD_DENY_FILE`. Every later SSH and SFTP invocation for
that connection pins the admitted address with `HostName` and disables
`ProxyJump` and `ProxyCommand`, so configuration changes or `Match exec` cannot
redirect it. `ruxel --version` reports how many
entries were compiled. This is a deny list only: the invoking environment is
still responsible for target admission and its own allow policy. Check and diff connect and inspect remote state. Only operate on explicitly authorized targets, and use isolated fixtures
for development. Never commit real credentials, host identities, or sensitive
captures to this public repository.

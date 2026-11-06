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
   handlers, and reports per-host results. Plan lists selected operations without connecting. Check and diff use
   non-mutating checks where the operation supports them; a task with
   `checkMode = false` can execute changes in either mode. Apply performs changes.

The CLI supports JSON and human-readable output. Optional JSONL event logs are
created as new private files. Treat generated output as sensitive: configuration
dumps can contain values supplied directly in Pkl, and secret redaction must be
verified for the workflow in use.

## Scope and safety

Schema definitions describe the accepted configuration surface; implementation
and regression evidence must be checked separately. The repository's local
acceptance suite exercises synthetic scenarios, but passing a subset does not
prove all providers or remote platforms are complete.

SSH uses strict host-key checking by default. The invoking environment is
responsible for target admission and its own guard policy; do not assume the
CLI alone supplies an allowlist. Check and diff connect and inspect remote state. Only operate on explicitly authorized targets, and use isolated fixtures
for development. Never commit real credentials, host identities, or sensitive
captures to this public repository.

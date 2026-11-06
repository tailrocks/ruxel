# Ruxel

Ruxel is a Rust automation controller and agent for configuring hosts from typed
[Pkl](https://pkl-lang.org/) modules. A controller evaluates a local module,
validates the resulting inventory and tasks, and runs operations through an
agent. The project is pre-1.0 and under active development; interfaces and
behavior can change.

The current migration replaces the former YAML and INI input with Pkl. The
schema covers host configuration, storage, PostgreSQL, controller operations,
expressions, and secret references. Coverage and end-to-end verification are
still in progress; the presence of a schema class alone does not establish that
an operation is verified on every platform.

## Build and check

Use the pinned tools in `mise.toml`:

```sh
mise run install
mise run build
mise run check
```

`mise run agent` builds a static x86-64 Linux agent. `mise run acceptance-local`
runs the local synthetic acceptance suite. It uses disposable local files and
processes; inspect the suite before running it on a machine with valuable data.

## Configuration

Create a module at the repository root, for example `example.pkl`:

```pkl
import "schema/Ruxel.pkl" as R

config = new R.Config {
  inventory {
    hosts { ["fixture"] = new R.Host { address = "localhost" } }
  }
  plays {
    new R.Play {
      name = "Example"
      targets { "fixture" }
      tasks {
        new R.Task {
          name = "Write a fixture file"
          operation = new R.Copy {
            dest = "/tmp/ruxel-example.txt"
            content = "hello\n"
            mode = "0644"
          }
        }
      }
    }
  }
}
```

The `config` property contains an `R.Config`. Hosts and groups are selected by
name in each play. Tasks can use typed operations, conditions, loops, registered
results, handlers, and blocks. See [the schema](schema/Ruxel.pkl) for the
available fields and [the architecture](docs/ARCHITECTURE.md) for evaluation
and execution details. `R.Lookup` describes a secret or external lookup; keep
secret values out of Pkl files and logs.

Inspect evaluated configuration without executing tasks:

```sh
ruxel config dump example.pkl
```

`config dump` prints evaluated values. Do not use it to print secrets. Evaluation
uses local Pkl files with an empty evaluator environment; runtime lookups are
resolved by the controller.

## Run

```sh
ruxel plan example.pkl --agent /path/to/ruxel-agent
ruxel check example.pkl --agent /path/to/ruxel-agent
ruxel diff example.pkl --agent /path/to/ruxel-agent
ruxel apply example.pkl --agent /path/to/ruxel-agent
```

`plan` lists selected operations without connecting or resolving runtime values.
`check` and `diff` connect to targets and request non-mutating operation checks;
`apply` executes changes. Operation support for check and diff varies, so verify a new
workflow on disposable targets first. `--hosts` and `--tags` select hosts and
tasks, `--jobs` controls concurrency, and `--output human` selects readable
progress. `--known-hosts` and `--ssh-key` configure SSH; strict host-key
checking is the default. Use `--accept-new-host-key` only when the applicable
SSH policy permits it. `--synthetic-secrets` supplies deterministic synthetic
lookup values for tests.

The local agent mode is for disposable local workloads only. It requires both
`--local` and `--state-dir`; operations can modify the local machine. Do not
run a configuration against a target unless you are authorized to do so and
have applied the guard policy required by your environment. `check` and `diff` still contact targets.

Exit status is `0` on success, `2` for command-line usage, `4` for invalid
configuration, and `8` for failed tasks. `--detailed-exitcode` returns `3` when
successful execution reports changes. Internal errors return `1`. Other
reserved codes are defined in `crates/ruxel/ruxel/src/exit.rs`.

## Development

Install hooks with `mise exec -- hk install --mise`. See
[operator setup](docs/OPERATOR-SETUP.md) for isolated fixture guidance and
[handoff](docs/RESTORE.md) for verification status. Do not put real host
identities or secrets in this public repository.

## License

[Apache-2.0](LICENSE)

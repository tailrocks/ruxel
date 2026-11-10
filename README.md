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

Create a module, for example `example.pkl`:

```pkl
import "modulepath:/ruxel/engine/Engine.pkl" as R

config = new R.Config {
  inventory {
    hosts { ["fixture"] = new R.Host { address = "127.0.0.1" } }
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
results, handlers, and blocks. See [the engine schema](schema/ruxel/engine/Engine.pkl) for the
available fields and [the architecture](docs/ARCHITECTURE.md) for evaluation
and execution details. `R.Lookup` describes a secret or external lookup; keep
secret values out of Pkl files and logs.

### The modules come from the binary

Every `ruxel` binary serves the Pkl modules it was built with under
`modulepath:/ruxel/`: workflows amend `modulepath:/ruxel/Workflow.pkl`, which
extends `Core.pkl`, and import the layer modules (`Linux.pkl`, `Systemd.pkl`,
`Debian.pkl`) for the system-specific steps they use; recipes and inventories
extend `modulepath:/ruxel/Core.pkl`; configurations written against the engine
import `modulepath:/ruxel/engine/Engine.pkl`. Configuration never
contains a copy of a Ruxel module. `ruxel plan`, `check`, `diff` and `apply` admit only a
configuration built from the module this process serves; they refuse one
evaluated against any copy, even an export of the same binary, and name the
schema fingerprints in the error.

Share roles and other Pkl libraries the same way. Pass directories with
`--module-path DIR[:DIR...]`, as with `pkl eval --module-path`, and import their
modules as `modulepath:/<path inside the directory>`. The directories are
searched in order after the compiled-in modules, so no directory can replace
anything under `modulepath:/ruxel/`.

A configuration can import and `read()` only files beneath its root
directory and the `--module-path` directories. The root directory is the root
module's directory unless `--root-dir DIR` names a wider one, as with
`pkl eval --root-dir`; use it when a root module imports `../` siblings. Paths
that leave these directories, absolute `file:` paths outside them, and symlinks
inside them are refused. Environment variables, external properties,
network imports, and module and resource globs (`import*`, `read*`) are not
available.

For editors and the `pkl` CLI, export the schema into an ignored directory and
pass that directory as the module path:

```sh
ruxel schema export .ruxel/module-path
pkl eval --module-path .ruxel/module-path example.pkl
```

The export is for editors and the `pkl` CLI only: `ruxel` refuses a
configuration that imports it by path. Run the export again after you upgrade
`ruxel`.

Inspect evaluated configuration without executing tasks:

```sh
ruxel config dump example.pkl
```

`config dump` prints evaluated values. Do not use it to print secrets. Evaluation
uses local Pkl files with an empty evaluator environment; runtime lookups are
resolved by the controller.

`Copy.src` supports regular controller files up to an aggregate 32 MiB per task
request, including loop iterations. Sources already larger than the remaining
limit are rejected before reading their content; reads remain bounded if a file
grows after its size is checked. Every task plan must also fit the 64 MiB protocol
frame limit.

## Run

```sh
ruxel plan example.pkl --agent /path/to/ruxel-agent
ruxel check example.pkl --agent /path/to/ruxel-agent
ruxel diff example.pkl --agent /path/to/ruxel-agent
ruxel apply example.pkl --agent /path/to/ruxel-agent
```

`plan` lists selected operations without connecting or resolving runtime values.
`check` and `diff` connect to targets and request non-mutating operation checks;
`apply` executes changes. A task with `checkMode = false` overrides check mode
and can execute changes during `check` or `diff`; the default and `true` request
a non-mutating check. Operation support for check and diff varies, so verify a
new workflow on disposable targets first. `--hosts` and `--tags` select hosts and
tasks, `--jobs` controls concurrency, and `--output human` selects readable
progress. `--known-hosts` and `--ssh-key` configure SSH; strict host-key
checking is the default. Use `--accept-new-host-key` only when the applicable
SSH policy permits it. `--synthetic-secrets` supplies deterministic synthetic
lookup values for tests. `--log-file` writes redacted JSONL events to a new
private file and refuses to overwrite an existing file.

The local agent mode is for disposable local workloads only. It requires both
`--local` and `--state-dir`; operations can modify the local machine. Do not
run a configuration against a target unless you are authorized to do so and
have applied the guard policy required by your environment. `check` and `diff` still contact targets.

Exit status is `0` on success, `2` for command-line usage, `4` for invalid
configuration, and `8` for failed tasks. `--detailed-exitcode` returns `3` when
successful execution reports changes; in plan mode, selected potential
operations also produce `3`. Internal errors return `1`. Other
reserved codes are defined in `crates/ruxel/ruxel/src/exit.rs`.

## Development

Install hooks with `mise exec -- hk install --mise`. See
[operator setup](docs/OPERATOR-SETUP.md) for isolated fixture guidance and
[handoff](docs/RESTORE.md) for verification status. Do not put real host
identities or secrets in this public repository.

## License

[Apache-2.0](LICENSE)

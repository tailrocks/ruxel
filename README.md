# Ruxel

> Rust-native automation without the YAML archaeology.

Ruxel is a drop-in, performance-first executor for the exact Ansible workload
that provisions and maintains the ChainArgos dedicated servers. Same playbook
files, same inventory, same check/diff/limit invocation shape — rebuilt in
Rust around one goal: **never repeat work that is already done, and prove it
in seconds.**

A converged server answers "0 changed, verified" in seconds instead of
~15 minutes. A drifted server gets exactly the drifted tasks re-applied.
A fresh server provisions with maximum parallelism. No Python on any target,
ever.

## Status

Implementation is feature-complete: the closed-surface modules, the
convergence ledger, the full `plan`/`apply` CLI, and the `op`-backed secret
resolver are built and verified, with 6 of 16 workload playbooks gated
three-way against pinned Ansible. Remaining work is verification breadth and
the M5 performance proof. Read the docs in order:

1. [docs/VISION.md](docs/VISION.md) — the problem, the vision, goals,
   non-goals, and the hard safety rule.
2. [docs/WORKLOAD.md](docs/WORKLOAD.md) — the closed compatibility spec:
   every module and feature the playbooks use, with counts. Ruxel implements
   this and nothing else.
3. [docs/DIRECTION.md](docs/DIRECTION.md) — problem analysis, prior art,
   recommended architecture, alternatives considered.
4. [docs/SKEPTIC.md](docs/SKEPTIC.md) — the adversarial pass: churn
   evidence, untuned-Ansible finding, NixOS/OpenTofu verdicts. Outcome:
   the operator decided to build the full drop-in.
5. [docs/SEMANTICS.md](docs/SEMANTICS.md) — **normative spec**: exactly
   what Ansible does with these files (param- and value-scoped, closed
   surface, ⚠-marked items pinned by parity experiments).
6. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — the detailed concept:
   transport decision (SSH as carrier, "gRPC minus the g" protocol),
   streaming execution, register-dependency pipelining, batched system
   caches, the convergence ledger, warm-daemon tier.
7. [docs/OPERATOR-SETUP.md](docs/OPERATOR-SETUP.md) — disposable fixture setup
   and the operator-only production boundary.
8. [docs/benchmarks/](docs/benchmarks/) — recorded benchmark evidence.
9. [docs/PLAN.md](docs/PLAN.md) — milestones M0–M6 with acceptance gates and
   the planned spec-drift CI watch.

## Hard safety rule

The six target hosts in the reference workload are **production**. Nothing in
this repository — code, tests, benchmarks, experiments — may ever connect to
them. Development happens exclusively against disposable targets explicitly
provided by the operator. See [AGENTS.md](AGENTS.md).

## Usage (target shape)

```bash
ruxel plan  -i hosts.ini --limit postgresql-nova setup-postgresql-nova.yml
ruxel apply -i hosts.ini --limit postgresql-nova setup-postgresql-nova.yml
```

## Development

Toolchain is pinned via [`rust-toolchain.toml`](rust-toolchain.toml); extra dev
tools are managed with [mise](https://mise.jdx.dev):

```bash
mise install
cargo build
just check
```

Releases are lightweight while the project remains a single-operator pilot:
update the workspace version and changelog, commit, then create an annotated
milestone tag (`git tag -a vX.Y.Z -m "ruxel X.Y.Z"`). `ruxel --version`
embeds that workspace version, so a deployed binary has a provenance handle.
Publishing to crates.io is intentionally unsupported; build a selected commit
or tag locally. Release asset automation can be added if distribution becomes
a project goal.

The private workload fidelity gates require `RUXEL_WORKLOAD_DIR` to point at
the `ansible-configs` checkout. They are explicitly ignored when unavailable;
run them locally with:

```bash
RUXEL_WORKLOAD_DIR=/path/to/ansible-configs \
  cargo test -p ruxel-core --test workload -- --ignored
RUXEL_WORKLOAD_DIR=/path/to/ansible-configs \
  cargo test -p ruxel-core --test render_parity template_files_match_oracle -- --ignored
```

## License

[Apache-2.0](LICENSE)

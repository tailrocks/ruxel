# Rust acceptance wiring candidate

This isolated WIP replaces 29 acceptance invocations with reviewed standalone
Rust crates. Every crate runs both `cargo test` and its actual local CLI/agent
runner. Building a runner alone does not establish acceptance. The former driver
sources remain until coordinated removal; these tasks no longer select them.

| Rust crate under `tools/` | Replaced drivers and unit modules under `tests/acceptance/pr66/` |
| --- | --- |
| `acceptance-plan` | `test_coverage`, `coverage`, `plan_matrix` |
| `acceptance-manifest` | `test_expectations_runner`, `expectations_runner`, both `local_scenarios` selections |
| `acceptance-pty` | `test_pause_pty`, `pause_pty` |
| `acceptance-control` | `retry`, `check_semantics`, `check_overrides` |
| `acceptance-files` | `files_lifecycle`, `template_source` |
| `acceptance-copy` | `copy_binary`, `copy_force`, `repeated_actions` |
| `acceptance-cache` | `cache`, `secret_provenance` |
| `acceptance-isolation` | `host_isolation`, `handshake_isolation`, `post_handshake_isolation`, `partial_recap` |
| `acceptance-runtime` | `control_runtime`, `handler_failure`, `nested_loops`, `scoped_variables`, `cross_play_facts`, `loop_task_vars` |

The manifest runner selects exactly `check-semantics`, `multihost` and
`controller-delegation`. Its default selection still fails for unsupported
scenarios. Fourteen successful plans and structural checks do not prove 14
behavioral lifecycles. Each runner README describes its boundaries. The separate
control-flow observation library is not a scenario adapter.

The manifest runner also preserves the former local-scenario assertions: exact
plan recap host sets, group/glob/comma/unmatched host selectors, skipped results
with `changed=false`, and snapshots of the complete staged fixture/schema tree
excluding agent state. These are assertion checks, not additional scenario adapters.

`deny-ci` uses the native `ruxel-config` `check-deny-inputs` binary before
cargo-deny. Named checks declare Rust sources, manifests, lockfiles, fixtures and
frozen oracle inputs explicitly.

## Pending XFS and aggregate gate

The existing XFS helper test invocation remains required, unchanged and marked
pending in `acceptance-local`. It was neither disabled nor run. The XFS runtime
also still embeds its previous helper. Consequently `acceptance-local`, the
aggregate `check`, and hosted CI must not be invoked under the operator's ban on
Python. Workflow regeneration is source generation only; it does not execute
acceptance or authorize a PR-head push.

The production default build still has native dependency work to resolve. Safe
local verification uses matching controller/agent artifacts, locked offline
standalone Cargo commands, and disabled C compiler commands. No server or device
action is part of this candidate.

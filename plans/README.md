# Active Implementation Plan

This file contains only work still required before Ruxel can be called a
verified Ansible-compatible executor for the closed workload in
`docs/WORKLOAD.md`. Completed audit plans remain in this directory as history;
they are not active work.

## Non-negotiable boundary

- Ansible 2.21 is the behavioral oracle.
- Real ChainArgos configuration is read-only input to offline feature
  extraction. Never execute, render-compare, or remotely apply those files.
- All executable comparison uses repository-owned synthetic files under
  `tools/fixture-project/` on disposable local or Hetzner demo servers.
- Never connect to, resolve, probe, or derive runtime inventory from production
  hosts.
- Unknown workload surface remains a hard error. Do not add speculative Ansible
  features outside the extracted closed surface.

## Completion definition

Ruxel is complete only when every extracted workload shape has a reproducible
chain of evidence:

`offline extraction -> synthetic fixture -> Ansible capture -> Ruxel result -> observable state diff`

The comparison must cover rendered values, task status, changed/skipped/failed
identity, registered result shape, diff output, check mode, first apply,
converged rerun, and final server state. Matching aggregate recap counts alone
is insufficient.

## Required work, in order

### 1. Build coverage traceability

- Extend the offline extractor to emit a versioned manifest of every required
  module, parameter/value, expression, template construct, lookup, loop,
  condition, register shape, block/handler/tag behavior, check-mode behavior,
  delegation, become/environment case, and diff surface.
- Map each manifest entry to one or more synthetic fixture tasks, oracle
  captures, Ruxel tests, and state assertions.
- Fail CI when an extracted shape lacks a fixture or verification mapping.
- Keep private workload access optional and offline; CI may consume a sanitized
  manifest artifact, never execute the source project.

**Done when:** the manifest reports 100% mapped coverage with no manual
exceptions and detects an intentionally removed fixture mapping.

### 2. Restore exhaustive controller/render parity

- Rebuild the removed Ansible-vs-Ruxel render harness using synthetic
  expressions, variables, templates, and dry secrets.
- Cover every extracted expression/template/control-flow input shape, including
  native types, undefined behavior, filters, lookup memoization boundaries,
  loop/register access, and error parity.
- Store reproducible golden inputs and normalized outputs in the repository.
- Resolve every remaining `⚠ verify` item in `docs/SEMANTICS.md` by measurement
  against pinned Ansible and record the experiment.

**Done when:** all mapped render cases compare byte-identically or have an
explicit documented intentional deviation; no unresolved `⚠ verify` remains.

### 3. Complete the synthetic fixture project

- Add synthetic playbooks/assets for all 36 closed-surface modules and every
  mapped control-flow shape.
- Include handlers, notify ordering, blocks/rescue, tags/always, pause,
  check-mode predictions, diffs, delegation, `become_user`, environment,
  retries/until, ignored failures, no-log, secret lookups, file/template
  rendering, packages/services, users/keys, firewall, Git, storage, and all
  PostgreSQL shapes.
- Model full playbook interactions where ordering matters; isolated unit-shaped
  fixtures alone are insufficient.
- Remove or replace captures whose source fixture is absent or outside
  `tools/fixture-project/`.

**Done when:** every traceability entry points to a committed, reproducible
synthetic fixture and every fixture parses/compiles under both tools.

### 4. Strengthen the executable parity oracle

- Replace changed-count-only gating with normalized per-task comparison:
  identity, status, changed, skipped, failed, rescued, ignored, result fields,
  redaction, and diff.
- Add automated final-state comparators for managed file bytes/metadata,
  packages/repos, units, users/groups/keys, sysctl, iptables, Git checkout,
  mounts/LVM/filesystems, and PostgreSQL catalog/ACL/ownership state.
- Test fresh Ansible and fresh Ruxel on equivalent disposable servers, plus both
  converged reruns and check/diff runs.
- Make capture regeneration deterministic and document exact commands.

**Done when:** every fixture passes fresh-state equivalence, first-run task
parity, converged parity, check/diff parity, and final-state equivalence.

### 5. Enforce disposable-target safety structurally

- Gate scripts must accept fixture identity, not arbitrary IP/inventory.
- Resolve addresses internally from the isolated fixture provider context and
  require the `ruxel=fixture` label plus an expected per-run identifier.
- Reject caller-supplied remote inventories and unknown targets before SSH or
  Ansible starts.
- Add hermetic tests proving non-fixture paths and targets are rejected.

**Done when:** normal parity tooling cannot be pointed at an arbitrary or
production target, even by argument mistake.

### 6. Finish multi-host transport and execution

- Keep plan 022's diagnostic rule: do not invent a transport fix for a stall
  that cannot be reproduced.
- Independently implement bounded parallel host execution with isolated
  connection state, deterministic output ordering, correct recap aggregation,
  and failure exit codes, unless investigation proves it depends on the stall.
- Verify sequential and concurrent connections on at least two disposable demo
  servers. Capture a six-host disposable-fixture benchmark.

**Done when:** multi-host runtime is approximately the slowest host rather than
the sum, output/recaps match Ansible semantics, and repeated fixture runs show
no transport stall or leaked resources.

### 7. Complete performance and resilience proof

- Benchmark Ansible and Ruxel on the same synthetic fixtures and equivalent
  disposable servers: fresh provision, converged no-op, one-task drift,
  check/diff, secret resolution, storage/PostgreSQL workloads, simulated RTT,
  and six-host parallel execution.
- Store raw logs, tool versions, fixture specification, repetitions, summary
  statistics, and correctness results beside each benchmark.
- Gate performance regressions only after correctness parity passes.
- Complete disconnect/kill chaos coverage at every protocol state and prove the
  disposable target is reusable afterward.

**Done when:** all benchmark classes are reproducible, Ruxel results remain
Ansible-equivalent, stated performance targets pass, and every chaos state
recovers.

## Not part of this active plan

- Production execution or production pilot. Only the operator can authorize
  that separately, host by host.
- Real-workload execution on demo servers. Feature extraction only.
- Warm daemon, drift dashboard, or unsupported Ansible surface.
- Optimizations without a correctness-preserving parity test.

## Current status

Core execution is substantially implemented and local quality gates are green.
Compatibility verification is incomplete. Workstreams 1-7 are open; plan 022
is incorporated into workstream 6. Do not describe Ruxel as complete until all
seven done criteria pass.

# PLAN — Remaining Work to Verified Ansible Compatibility

Status: active. Implementation core exists; compatibility proof is incomplete.
The executable backlog is maintained in [`plans/README.md`](../plans/README.md).

## Source of truth

Pinned Ansible 2.21 defines behavior for the closed surface extracted from the
16 reference playbooks. Real reference configuration is used only for offline
feature extraction. Every executable experiment uses repository-owned
synthetic fixtures on disposable demo servers.

Production hosts are never contacted, resolved, probed, or used for automated
comparison. A future production pilot is a separate operator-authorized phase
and is not a completion gate for this development plan.

## Remaining milestones

1. Finish fresh/converged/check/diff task and final-state parity for every
   synthetic fixture, including fully provisioned storage and PostgreSQL cases.
2. Publish reproducible correctness-coupled performance and chaos evidence for
   fresh, converged, drifted, secretful, storage, PostgreSQL, RTT, and
   multi-host cases.

## Final acceptance gate

Ruxel may be called complete only when:

- every extracted feature shape is mapped and tested;
- pinned Ansible and Ruxel agree on rendered inputs, task outcomes, result
  shapes, diffs, and final disposable-server state;
- every closed-surface module and control-flow interaction has executable
  synthetic evidence;
- all experiments are reproducible from committed fixture sources;
- all remaining semantic questions are resolved by oracle measurement;
- multi-host execution and every required benchmark/chaos class pass; and
- normal tooling cannot accidentally target a non-fixture server.

Until then the correct status is: **core implemented; Ansible compatibility
verification incomplete**.

# Ruxel fixture project

This is the only playbook project allowed for remote Ruxel/Ansible parity
runs. It models the feature shapes extracted offline from the real workload,
using synthetic names, paths, credentials, services, and data.

Hard boundary:

- Real workload files may be read only by offline spec-extraction tests.
- Real playbooks, inventory, templates, secrets, hostnames, and device IDs are
  never passed to `ruxel apply` or `ansible-playbook`.
- `tools/fixtures/bless-gate.sh` and `tools/oracle/capture_fixture.sh` reject
  every playbook outside this directory.
- Inventories contain only disposable resources created in the isolated
  `ruxel-fixtures` Hetzner project.

Each playbook should isolate a compatibility surface while preserving the
same Ansible syntax and observable semantics. Ansible 2.21 is the oracle;
Ruxel fresh apply, Ruxel converged rerun, and Ansible bless must agree.

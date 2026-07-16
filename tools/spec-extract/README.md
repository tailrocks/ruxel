# Spec-drift extractor

Checks YAML playbooks/task files against `ruxel_core::modules::MODULES` and
exits nonzero for unknown modules, parameters, or closed literal values.

```sh
cargo run -p ruxel-spec-extract -- /path/to/ansible-configs
```

Exit codes: `0` no drift, `1` uncovered surface, `2` usage/read/YAML error.
The unit test injects one unknown module, parameter, and literal value.

CI wiring remains pending because the live `ChainArgos/ansible-configs`
checkout is private. Provide its read-only deploy credential to CI, set
`RUXEL_WORKLOAD_DIR`, then run this tool. Never embed that credential or copy
private workload content into this repository.

# Spec-drift extractor

Checks YAML playbooks/task files against `ruxel_core::modules::MODULES` and
exits nonzero for unknown modules, parameters, or closed literal values. It
also extracts a normalized, value-free feature manifest and proves the
repository-owned synthetic project covers every observed shape.

```sh
cargo run -p ruxel-spec-extract -- /path/to/ansible-configs
cargo run -p ruxel-spec-extract -- manifest /path/to/ansible-configs workload-features.json
cargo run -p ruxel-spec-extract -- verify workload-features.json ../fixture-project
```

Exit codes: `0` no drift, `1` uncovered surface, `2` usage/read/YAML error.
`workload-features.json` contains only module/parameter identifiers, safe enum
values, value types, and normalized control/template shapes. It contains no
playbook paths, task names, hostnames, arbitrary values, or secrets. Regenerate
it only from the local offline reference checkout, inspect the diff, then
commit it. CI needs no access to that checkout: it verifies the committed
manifest against `tools/fixture-project/`.

The unit tests inject unknown surface and prove private mapping names and shell
pipes are not mistaken for compatibility features. Never embed a private
credential or copy private workload content into this repository.

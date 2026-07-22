# Oracle

A pinned **ansible-core 2.21** environment used as the reference oracle
during development: Ansible runs of repository-owned synthetic fixture
playbooks produce golden captures (per-task
rendered args, results, statuses, diffs) that ruxel's behavior is diffed
against. Test-time tooling only — Python never appears in the ruxel product
(controller and agent are Rust; targets never run Python).

Only playbooks under `../fixture-project/` may be executed. Real workload
configuration is offline extraction input only. `capture_fixture.sh` enforces
this path boundary.

```bash
cd tools/oracle && uv sync          # create the pinned venv

ANSIBLE_CALLBACK_PLUGINS=callback_plugins \
ANSIBLE_CALLBACKS_ENABLED=ruxel_capture \
RUXEL_CAPTURE_FILE=/tmp/capture.jsonl \
uv run ansible-playbook -i <inventory> <playbook>.yml
```

Captured records are JSON lines; see `callback_plugins/ruxel_capture.py`.
Note: by the time results reach the callback, `raw_args` are already
template-rendered (verified on 2.21.0) — captures carry post-template
parameters even for modules that do not echo an `invocation`.

## Offline render parity

`render_parity.py` walks every repository-owned synthetic playbook and template
without loading inventory or opening a connection. Pinned Ansible renders each
expression, condition, loop bind, and template file with deterministic fake
facts/registers/lookups. The committed JSONL is replayed through Ruxel by
`crates/ruxel-core/tests/render_parity.rs`.

```bash
cd tools/oracle
uv run python render_parity.py
git diff --exit-code captures/render-parity.jsonl
cargo nextest run -p ruxel-core --test render_parity
```

The corpus contains synthetic fixture names and values only. Regeneration from
the real workload is forbidden.

## Remote result and state parity

`tools/fixtures/bless-gate.sh` emits Ruxel JSON events containing normalized
task results, captures the subsequent Ansible run, and compares task identity,
module, status, ignored state, registered-result fields, and diffs with
`compare_results.py`. It snapshots managed files/metadata/content, selected
packages/accounts/sysctls/firewall/mount/LVM state, and synthetic PostgreSQL
catalog state before and after the Ansible bless; any state mutation fails the
gate.

Remote tools accept only a labeled provider fixture name. Raw IPs and caller
inventories are rejected before SSH starts.

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

#!/usr/bin/env python3
"""Render-parity capture: drive ansible-core 2.21's real Templar over every
template-bearing string in the workload (playbook expressions, conditions,
loop sources, free-form bodies) and every template file, with deterministic
fake variables (parity_vars.json) and dry-secret fake lookups. Output is a
JSONL golden corpus that crates/ruxel-core/tests/render_parity.rs replays
byte-for-byte through ruxel's engine.

Usage:
    cd tools/oracle
    RUXEL_WORKLOAD_DIR=~/path/to/ansible-configs uv run python render_parity.py

Offline by construction: lookups are fakes, no inventory, no connections.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
from pathlib import Path

HERE = Path(__file__).parent

# Fake plugin wiring must precede any ansible import.
os.environ["ANSIBLE_COLLECTIONS_PATH"] = str(HERE / "collections")
os.environ["ANSIBLE_LOOKUP_PLUGINS"] = str(HERE / "lookup_plugins")
os.environ.setdefault("ANSIBLE_LOCALHOST_WARNING", "False")
os.environ.setdefault("ANSIBLE_DEPRECATION_WARNINGS", "False")

from ansible.plugins.loader import init_plugin_loader  # noqa: E402

init_plugin_loader()

import jinja2  # noqa: E402  (the oracle venv's jinja, for undeclared-name scanning)
import jinja2.meta  # noqa: E402
from ansible.parsing.dataloader import DataLoader  # noqa: E402
from ansible.template import Templar, trust_as_template  # noqa: E402
from ansible import errors as ansible_errors  # noqa: E402

CONDITION_KEYS = ("when", "changed_when", "failed_when", "until")
TASK_CONTROL_KEYS = {
    "name", "when", "register", "loop", "loop_control", "vars", "tags",
    "notify", "become", "become_user", "changed_when", "failed_when",
    "ignore_errors", "check_mode", "no_log", "environment", "until",
    "retries", "delay", "args", "block", "rescue", "always",
}
# Names jinja2.meta reports that are not workload variables.
BUILTIN_NAMES = {"lookup", "item", "true", "false", "none", "True", "False", "None"}
# Referenced by config/sentry/config.yml but defined nowhere in the workload:
# a real ansible-playbook run errors with AnsibleUndefinedVariable when it
# templates that file. The error IS the golden; if these ever gain a
# definition, the captures change and the parity gate flags it.
UNDEFINED_IN_WORKLOAD = {"slack_client_id", "slack_client_secret", "slack_signing_secret"}

records: list[dict] = []
undeclared: set[str] = set()
scan_errors: set[str] = set()
jinja_env = jinja2.Environment()
# find_undeclared_variables runs codegen, which validates filter names;
# stub the ansible-side filters so scanning never fails on them.
for _name in ("bool", "hash", "subelements", "b64decode"):
    jinja_env.filters.setdefault(_name, lambda *a, **kw: None)


def has_template(s: str) -> bool:
    return "{{" in s or "{%" in s


def scan_names(expr_or_template: str, available: dict) -> None:
    try:
        ast = jinja_env.parse(expr_or_template)
        names = jinja2.meta.find_undeclared_variables(ast)
    except jinja2.TemplateAssertionError as e:
        # Unknown filter/test = a construct outside the stubbed surface;
        # surface it loudly instead of silently skipping the name scan.
        scan_errors.add(f"{e} in {expr_or_template[:80]!r}")
        return
    except jinja2.TemplateSyntaxError:
        return
    for name in names:
        if name not in available and name not in BUILTIN_NAMES and name not in UNDEFINED_IN_WORKLOAD:
            undeclared.add(name)


def encode_result(value) -> dict:
    if isinstance(value, str):
        return {"t": "str", "v": str(value)}
    return {"t": "native", "v": json.loads(json.dumps(value, default=str))}


def emit(kind: str, playbook: str, task: str, field: str, input_str: str,
         bind: dict | None, result: dict) -> None:
    records.append({
        "kind": kind,
        "playbook": playbook,
        "task": task,
        "field": field,
        "input": str(input_str),
        "bind": bind,
        "result": result,
    })


def capture_template(templar: Templar, playbook: str, task: str, field: str,
                     value: str, bind: dict | None) -> None:
    scan_names(value, templar.available_variables)
    try:
        rendered = templar.template(trust_as_template(str(value)))
        result = encode_result(rendered)
    except Exception as e:  # noqa: BLE001 — error parity is part of the contract
        result = {"t": "error", "v": type(e).__name__}
    emit("expr", playbook, task, field, value, bind, result)


def capture_condition(templar: Templar, playbook: str, task: str, field: str,
                      expr, bind: dict | None) -> None:
    if isinstance(expr, bool):
        return  # literal conditions have no render semantics to pin
    scan_names("{{ (" + str(expr) + ") }}", templar.available_variables)
    try:
        result = {"t": "bool", "v": bool(templar.evaluate_conditional(trust_as_template(str(expr))))}
    except Exception as e:  # noqa: BLE001
        result = {"t": "error", "v": type(e).__name__}
    emit("condition", playbook, task, field, str(expr), bind, result)


def walk_params(templar: Templar, playbook: str, task: str, prefix: str,
                value, bind: dict | None) -> None:
    if isinstance(value, str):
        if has_template(value):
            capture_template(templar, playbook, task, prefix, value, bind)
    elif isinstance(value, dict):
        for k, v in value.items():
            walk_params(templar, playbook, task, f"{prefix}.{k}", v, bind)
    elif isinstance(value, list):
        for i, v in enumerate(value):
            walk_params(templar, playbook, task, f"{prefix}[{i}]", v, bind)


def listify(value) -> list:
    return value if isinstance(value, list) else [value]


def process_task(loader: DataLoader, playbook: str, base_vars: dict, task: dict) -> None:
    task_name = str(task.get("name", "(unnamed)"))
    for section in ("block", "rescue", "always"):
        if section in task:
            # Block keywords (when, become, tags) inherit; conditions on the
            # block itself are captured with base vars.
            block_templar = Templar(loader=loader, variables=base_vars)
            for key in CONDITION_KEYS:
                if key in task:
                    for i, expr in enumerate(listify(task[key])):
                        capture_condition(block_templar, playbook, task_name,
                                          f"block.{key}[{i}]", expr, None)
            for sub in task[section]:
                process_task(loader, playbook, base_vars, sub)
    if "block" in task:
        return

    task_vars = dict(base_vars)
    for k, v in (task.get("vars") or {}).items():
        task_vars[k] = v
    templar = Templar(loader=loader, variables=task_vars)

    # Loop source first: it is itself a template with native-list semantics.
    binds: list[dict | None] = [None]
    if "loop" in task:
        loop_value = task["loop"]
        if isinstance(loop_value, str):
            capture_template(templar, playbook, task_name, "loop", loop_value, None)
        try:
            items = templar.template(trust_as_template(loop_value) if isinstance(loop_value, str) else loop_value)
        except Exception:  # noqa: BLE001
            items = []
        if isinstance(items, list) and items:
            # Bind the live (lazily-templated) item for the oracle templar;
            # record a JSON snapshot (which may contain raw inner templates —
            # the Rust replay binds it as a Raw layer and re-renders).
            binds = [
                {"item": item, "_record": {"item": json.loads(json.dumps(item, default=str))}}
                for item in items[:2]
            ]

    module_keys = [k for k in task if k not in TASK_CONTROL_KEYS]
    for bind in binds:
        bound_vars = dict(task_vars)
        record_bind = None
        if bind:
            bound_vars["item"] = bind["item"]
            record_bind = bind["_record"]
        bound_templar = Templar(loader=loader, variables=bound_vars)

        for key in CONDITION_KEYS:
            if key in task:
                for i, expr in enumerate(listify(task[key])):
                    capture_condition(bound_templar, playbook, task_name,
                                      f"{key}[{i}]", expr, record_bind)

        for mod in module_keys:
            body = task[mod]
            if mod == "assert" and isinstance(body, dict):
                for i, expr in enumerate(listify(body.get("that", []))):
                    capture_condition(bound_templar, playbook, task_name,
                                      f"assert.that[{i}]", expr, record_bind)
                if "fail_msg" in body and has_template(str(body["fail_msg"])):
                    capture_template(bound_templar, playbook, task_name,
                                     "assert.fail_msg", body["fail_msg"], record_bind)
                continue
            if isinstance(body, str):
                if has_template(body):
                    capture_template(bound_templar, playbook, task_name,
                                     f"{mod}(free-form)", body, record_bind)
            else:
                walk_params(bound_templar, playbook, task_name, mod, body, record_bind)

        if "args" in task:
            walk_params(bound_templar, playbook, task_name, "args", task["args"], record_bind)
        if "environment" in task:
            walk_params(bound_templar, playbook, task_name, "environment",
                        task["environment"], record_bind)


def render_template_file(loader: DataLoader, workload: Path, playbook: str,
                         src: str, base_vars: dict) -> None:
    full = workload / src
    content = loader.get_text_file_contents(str(full))
    scan_names(content, base_vars)
    templar = Templar(loader=loader, variables=base_vars)
    overrides = {"trim_blocks": True, "lstrip_blocks": False, "newline_sequence": "\n"}
    try:
        rendered = templar.template(
            trust_as_template(content), escape_backslashes=False, overrides=overrides,
        )
        rendered_bytes = str(rendered).encode()
        result = {
            "t": "file",
            "sha256": hashlib.sha256(rendered_bytes).hexdigest(),
            "len": len(rendered_bytes),
            "tail_nl": rendered_bytes.endswith(b"\n"),
        }
    except Exception as e:  # noqa: BLE001
        result = {"t": "error", "v": type(e).__name__}
    records.append({
        "kind": "template_file",
        "playbook": playbook,
        "src": src,
        "result": result,
    })


def main() -> int:
    workload = Path(os.environ["RUXEL_WORKLOAD_DIR"]).expanduser()
    fakes = json.loads((HERE / "parity_vars.json").read_text())
    fakes.pop("_comment", None)

    loader = DataLoader()
    loader.set_basedir(str(workload))

    playbooks = sorted(p for p in workload.iterdir() if p.suffix == ".yml")
    assert playbooks, f"no playbooks under {workload}"

    import ansible
    records.append({
        "kind": "meta",
        "ansible": ansible.__version__,
        "playbooks": len(playbooks),
    })

    template_srcs: list[tuple[str, str, dict]] = []

    for pb_path in playbooks:
        pb_name = pb_path.name
        plays = loader.load_from_file(str(pb_path), trusted_as_template=True)
        for play_idx, play in enumerate(plays):
            play_vars = dict(play.get("vars") or {})
            play_vars.pop("ansible_python_interpreter", None)
            records.append({
                "kind": "playbook_vars",
                "playbook": pb_name,
                "play": play_idx,
                "vars": json.loads(json.dumps(play_vars, default=str)),
            })
            base_vars = dict(play_vars)
            base_vars.update(fakes)

            for section in ("pre_tasks", "tasks", "handlers"):
                for task in play.get(section) or []:
                    process_task(loader, pb_name, base_vars, task)
                    for t in iter_tasks(task):
                        mod_body = t.get("template")
                        if isinstance(mod_body, dict) and "src" in mod_body:
                            template_srcs.append((pb_name, str(mod_body["src"]), base_vars))

    seen: set[str] = set()
    for pb_name, src, base_vars in template_srcs:
        if src in seen:
            continue
        seen.add(src)
        render_template_file(loader, workload, pb_name, src, base_vars)

    if undeclared or scan_errors:
        print("FATAL: names with no fake in parity_vars.json and no play var:",
              file=sys.stderr)
        for name in sorted(undeclared):
            print(f"  {name}", file=sys.stderr)
        for err in sorted(scan_errors):
            print(f"  scan error: {err}", file=sys.stderr)
        return 1

    out = HERE / "captures" / "render-parity.jsonl"
    out.parent.mkdir(exist_ok=True)
    with out.open("w") as f:
        for rec in records:
            f.write(json.dumps(rec, sort_keys=True, ensure_ascii=False) + "\n")

    by_kind: dict[str, int] = {}
    for rec in records:
        by_kind[rec["kind"]] = by_kind.get(rec["kind"], 0) + 1
    print(f"wrote {out} ({len(records)} records): "
          + ", ".join(f"{k}={v}" for k, v in sorted(by_kind.items())))
    return 0


def iter_tasks(task: dict):
    yield task
    for section in ("block", "rescue", "always"):
        for sub in task.get(section) or []:
            yield from iter_tasks(sub)


if __name__ == "__main__":
    sys.exit(main())

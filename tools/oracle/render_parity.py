#!/usr/bin/env python3
"""Capture pinned Ansible rendering for repository-owned synthetic fixtures.

No inventory or remote connection is loaded. Lookups use deterministic fake
plugins. Output is replayed by crates/ruxel-core/tests/render_parity.rs.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

HERE = Path(__file__).parent
ROOT = HERE.parent.parent
FIXTURES = ROOT / "tools" / "fixture-project"

os.environ["ANSIBLE_COLLECTIONS_PATH"] = str(HERE / "collections")
os.environ["ANSIBLE_LOOKUP_PLUGINS"] = str(HERE / "lookup_plugins")
os.environ.setdefault("ANSIBLE_LOCALHOST_WARNING", "False")
os.environ.setdefault("ANSIBLE_DEPRECATION_WARNINGS", "False")

from ansible.plugins.loader import init_plugin_loader  # noqa: E402

init_plugin_loader()

from ansible.parsing.dataloader import DataLoader  # noqa: E402
from ansible.template import Templar, trust_as_template  # noqa: E402

CONDITION_KEYS = ("when", "changed_when", "failed_when", "until")
TASK_CONTROL_KEYS = {
    "name", "when", "register", "loop", "loop_control", "vars", "tags",
    "notify", "become", "become_user", "delegate_to", "changed_when",
    "failed_when", "ignore_errors", "check_mode", "no_log", "environment",
    "until", "retries", "delay", "args", "block", "rescue", "always",
}


def encode(value):
    if isinstance(value, str):
        return {"t": "str", "v": value}
    return {"t": "native", "v": json.loads(json.dumps(value, default=str))}


def capture_template(records, templar, playbook, task, field, value, bind):
    try:
        result = encode(templar.template(trust_as_template(str(value))))
    except Exception as error:  # error type is observable parity
        result = {"t": "error", "v": type(error).__name__}
    records.append({"kind": "expr", "playbook": playbook, "task": task,
                    "field": field, "input": str(value), "bind": bind,
                    "result": result})


def capture_condition(records, templar, playbook, task, field, expression, bind):
    if isinstance(expression, bool):
        return
    try:
        result = {"t": "bool", "v": bool(templar.evaluate_conditional(
            trust_as_template(str(expression))))}
    except Exception as error:
        result = {"t": "error", "v": type(error).__name__}
    records.append({"kind": "condition", "playbook": playbook, "task": task,
                    "field": field, "input": str(expression), "bind": bind,
                    "result": result})


def walk_values(records, templar, playbook, task, prefix, value, bind):
    if isinstance(value, str) and ("{{" in value or "{%" in value):
        capture_template(records, templar, playbook, task, prefix, value, bind)
    elif isinstance(value, dict):
        for key, child in value.items():
            walk_values(records, templar, playbook, task, f"{prefix}.{key}", child, bind)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            walk_values(records, templar, playbook, task, f"{prefix}[{index}]", child, bind)


def listify(value):
    return value if isinstance(value, list) else [value]


def process_task(records, loader, playbook, base_vars, task):
    task_name = str(task.get("name", "(unnamed)"))
    for section in ("block", "rescue", "always"):
        if section in task:
            templar = Templar(loader=loader, variables=base_vars)
            for key in CONDITION_KEYS:
                for index, expression in enumerate(listify(task.get(key, []))):
                    capture_condition(records, templar, playbook, task_name,
                                      f"block.{key}[{index}]", expression, None)
            for child in task[section]:
                process_task(records, loader, playbook, base_vars, child)
    if "block" in task:
        return

    task_vars = dict(base_vars)
    task_vars.update(task.get("vars") or {})
    templar = Templar(loader=loader, variables=task_vars)
    binds = [None]
    if "loop" in task:
        loop_value = task["loop"]
        if isinstance(loop_value, str):
            capture_template(records, templar, playbook, task_name, "loop", loop_value, None)
        try:
            items = templar.template(
                trust_as_template(loop_value) if isinstance(loop_value, str) else loop_value)
        except Exception:
            items = []
        if isinstance(items, list) and items:
            binds = [{"item": item} for item in items[:2]]

    for bind in binds:
        bound_vars = dict(task_vars)
        if bind:
            bound_vars.update(bind)
        bound = Templar(loader=loader, variables=bound_vars)
        for key in CONDITION_KEYS:
            for index, expression in enumerate(listify(task.get(key, []))):
                capture_condition(records, bound, playbook, task_name,
                                  f"{key}[{index}]", expression, bind)
        for module in (key for key in task if key not in TASK_CONTROL_KEYS):
            body = task[module]
            if module == "assert" and isinstance(body, dict):
                for index, expression in enumerate(listify(body.get("that", []))):
                    capture_condition(records, bound, playbook, task_name,
                                      f"assert.that[{index}]", expression, bind)
                walk_values(records, bound, playbook, task_name, module, body.get("fail_msg"), bind)
            else:
                walk_values(records, bound, playbook, task_name, module, body, bind)
        for field in (
            "name", "notify", "tags", "delegate_to", "become_user", "vars",
            "args", "environment", "loop_control",
        ):
            if field in task:
                walk_values(records, bound, playbook, task_name, field, task[field], bind)


def render_template_file(records, loader, playbook, source, variables):
    content = loader.get_text_file_contents(str(FIXTURES / source))
    templar = Templar(loader=loader, variables=variables)
    try:
        rendered = str(templar.template(
            trust_as_template(content), escape_backslashes=False,
            overrides={"trim_blocks": True, "lstrip_blocks": False,
                       "newline_sequence": "\n"}))
        data = rendered.encode()
        result = {"t": "file", "sha256": hashlib.sha256(data).hexdigest(),
                  "len": len(data), "tail_nl": data.endswith(b"\n")}
    except Exception as error:
        result = {"t": "error", "v": type(error).__name__}
    records.append({"kind": "template_file", "playbook": playbook,
                    "src": source, "result": result})


def iter_tasks(task):
    yield task
    for section in ("block", "rescue", "always"):
        for child in task.get(section) or []:
            yield from iter_tasks(child)


def main():
    loader = DataLoader()
    loader.set_basedir(str(FIXTURES))
    fakes = json.loads((HERE / "parity_vars.json").read_text())
    records = [{"kind": "meta", "ansible": __import__("ansible").__version__,
                "fixture_schema": 1}]
    templates = []
    for path in sorted(FIXTURES.glob("*.yml")):
        plays = loader.load_from_file(str(path), trusted_as_template=True)
        for play_index, play in enumerate(plays):
            play_vars = dict(play.get("vars") or {})
            records.append({"kind": "playbook_vars", "playbook": path.name,
                            "play": play_index, "vars": play_vars})
            variables = dict(play_vars)
            variables.update(fakes)
            walk_values(records, Templar(loader=loader, variables=variables),
                        path.name, "(play vars)", "vars", play_vars, None)
            for section in ("pre_tasks", "tasks", "handlers"):
                for task in play.get(section) or []:
                    process_task(records, loader, path.name, variables, task)
                    for nested in iter_tasks(task):
                        body = nested.get("template")
                        if isinstance(body, dict) and "src" in body:
                            templates.append((path.name, str(body["src"]), variables))
    seen = set()
    for playbook, source, variables in templates:
        if source not in seen and "{{" not in source:
            seen.add(source)
            render_template_file(records, loader, playbook, source, variables)
    output = HERE / "captures" / "render-parity.jsonl"
    with output.open("w") as stream:
        for record in records:
            stream.write(json.dumps(record, sort_keys=True, ensure_ascii=False) + "\n")
    print(f"wrote {output} ({len(records)} records, {len(seen)} template files)")


if __name__ == "__main__":
    main()

"""Ansible callback plugin that captures per-task oracle data for ruxel.

Records one JSON line per task/item result to the file named by
RUXEL_CAPTURE_FILE (default: ./ruxel-capture.jsonl): play, task name, action,
raw task args (pre-template), resolved module args (post-template, from the
module invocation when the module reports it), the full result dict, status,
and diff. no_log tasks arrive already censored by Ansible core - the capture
stores exactly what Ansible itself would log.

Enable with:
    ANSIBLE_CALLBACK_PLUGINS=tools/oracle/callback_plugins \
    ANSIBLE_CALLBACKS_ENABLED=ruxel_capture \
    RUXEL_CAPTURE_FILE=/tmp/capture.jsonl \
    ansible-playbook ...
"""

from __future__ import annotations

import json
import os
import threading

from ansible.plugins.callback import CallbackBase

DOCUMENTATION = """
    name: ruxel_capture
    type: notification
    short_description: capture per-task results as JSON lines for ruxel parity
    description:
        - Writes one JSON object per task result to RUXEL_CAPTURE_FILE.
    requirements:
        - enable in ansible.cfg or ANSIBLE_CALLBACKS_ENABLED
"""


def _jsonable(value):
    """Coerce Ansible's internal types (AnsibleUnicode, wrappers) to plain JSON."""
    try:
        return json.loads(json.dumps(value, default=str))
    except (TypeError, ValueError):
        return repr(value)


class CallbackModule(CallbackBase):
    CALLBACK_VERSION = 2.0
    CALLBACK_TYPE = "notification"
    CALLBACK_NAME = "ruxel_capture"
    CALLBACK_NEEDS_ENABLED = True

    def __init__(self):
        super().__init__()
        self._path = os.environ.get("RUXEL_CAPTURE_FILE", "ruxel-capture.jsonl")
        self._lock = threading.Lock()
        self._play = None
        self._playbook = None

    def _write(self, record):
        record["playbook"] = self._playbook
        record["play"] = self._play
        line = json.dumps(record, sort_keys=True, default=str)
        with self._lock:
            with open(self._path, "a", encoding="utf-8") as fh:
                fh.write(line + "\n")

    def _task_record(self, status, result, item=None):
        task = result._task
        raw = _jsonable(getattr(task, "args", {}))
        res = _jsonable(result._result)
        # Resolved (post-template) module args, when the module echoes its
        # invocation. Absent for some actions and for no_log tasks.
        invocation = res.get("invocation") if isinstance(res, dict) else None
        resolved = invocation.get("module_args") if isinstance(invocation, dict) else None
        record = {
            "status": status,
            "host": result._host.get_name(),
            "task_name": task.get_name(),
            "action": task.action,
            "raw_args": raw,
            "resolved_args": resolved,
            "result": res,
            "changed": bool(res.get("changed")) if isinstance(res, dict) else None,
            "diff": res.get("diff") if isinstance(res, dict) else None,
        }
        if item is not None:
            record["item_label"] = _jsonable(item)
        return record

    # -- lifecycle ---------------------------------------------------------

    def v2_playbook_on_start(self, playbook):
        self._playbook = getattr(playbook, "_file_name", None)

    def v2_playbook_on_play_start(self, play):
        self._play = play.get_name()

    # -- whole-task results ------------------------------------------------

    def v2_runner_on_ok(self, result):
        self._write(self._task_record("ok", result))

    def v2_runner_on_failed(self, result, ignore_errors=False):
        rec = self._task_record("failed", result)
        rec["ignore_errors"] = ignore_errors
        self._write(rec)

    def v2_runner_on_skipped(self, result):
        self._write(self._task_record("skipped", result))

    def v2_runner_on_unreachable(self, result):
        self._write(self._task_record("unreachable", result))

    # -- per-loop-item results ----------------------------------------------

    def v2_runner_item_on_ok(self, result):
        self._write(self._task_record("item_ok", result, item=result._result.get("item")))

    def v2_runner_item_on_failed(self, result):
        self._write(self._task_record("item_failed", result, item=result._result.get("item")))

    def v2_runner_item_on_skipped(self, result):
        self._write(self._task_record("item_skipped", result, item=result._result.get("item")))

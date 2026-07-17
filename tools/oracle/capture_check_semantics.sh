#!/bin/sh
# Pin check-mode pause + predicted-handler behavior using localhost only.
set -eu
cd "$(dirname "$0")/../.."

raw=$(mktemp)
trap 'rm -f "$raw"' EXIT

ANSIBLE_CALLBACK_PLUGINS=tools/oracle/callback_plugins \
ANSIBLE_CALLBACKS_ENABLED=ruxel_capture \
ANSIBLE_LOCALHOST_WARNING=False \
RUXEL_CAPTURE_FILE="$raw" \
tools/oracle/.venv/bin/ansible-playbook \
  -i 'localhost,' -c local --check \
  tools/fixture-project/check-semantics.yml </dev/null >/dev/null

python3 - "$raw" tools/oracle/captures/check-semantics.jsonl <<'PY'
import json
import sys

source, destination = sys.argv[1:]
records = []
for line in open(source, encoding="utf-8"):
    record = json.loads(line)
    if record["action"] == "gather_facts":
        continue
    result = record.get("result") or {}
    normalized = {
        "task": record["task_name"],
        "action": record["action"],
        "status": record["status"],
        "changed": bool(result.get("changed")),
    }
    for key in ("msg", "user_input"):
        if key in result:
            normalized[key] = result[key]
    records.append(normalized)

with open(destination, "w", encoding="utf-8") as output:
    for record in records:
        output.write(json.dumps(record, sort_keys=True) + "\n")
PY

echo "wrote tools/oracle/captures/check-semantics.jsonl"

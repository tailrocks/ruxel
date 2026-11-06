#!/bin/sh
set -eu
cd "$(dirname "$0")"

rm -f captures/runtime-semantics.jsonl

ANSIBLE_CALLBACK_PLUGINS=callback_plugins \
ANSIBLE_CALLBACKS_ENABLED=ruxel_capture \
ANSIBLE_LOCALHOST_WARNING=False \
RUXEL_CAPTURE_FILE=captures/runtime-semantics.jsonl \
uv run ansible-playbook -i 'localhost,' -c local runtime_semantics.yml "$@"

echo "wrote captures/runtime-semantics.jsonl ($(wc -l < captures/runtime-semantics.jsonl | tr -d ' ') records)"

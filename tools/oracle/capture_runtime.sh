#!/bin/sh
# Capture the runtime-semantics goldens: run runtime_semantics.yml against
# localhost (connection local — no remote target, ever) under the pinned
# ansible-core 2.21 with the ruxel_capture callback, writing
# captures/runtime-semantics.jsonl.
set -eu
cd "$(dirname "$0")"

ANSIBLE_CALLBACK_PLUGINS=callback_plugins \
ANSIBLE_CALLBACKS_ENABLED=ruxel_capture \
ANSIBLE_LOCALHOST_WARNING=False \
RUXEL_CAPTURE_FILE=captures/runtime-semantics.jsonl \
uv run ansible-playbook -i 'localhost,' -c local runtime_semantics.yml "$@"

echo "wrote captures/runtime-semantics.jsonl ($(wc -l < captures/runtime-semantics.jsonl | tr -d ' ') records)"

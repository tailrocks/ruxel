#!/bin/sh
# Capture a real ansible-core 2.21 run of a workload playbook against a
# ruxel fixture VM (tools/fixtures/create.sh output), writing golden
# records. Targets come exclusively from the fixture scripts — never the
# production inventory (GOAL.md rule 2).
#
# Usage:
#   tools/oracle/capture_fixture.sh <fixture-ip> <keyfile> <playbook-path> <capture-name>
set -eu
cd "$(dirname "$0")"

IP="${1:?fixture ip}"
KEY="${2:?ssh keyfile}"
PLAYBOOK="${3:?playbook path}"
NAME="${4:?capture name}"

INV="$(mktemp)"
trap 'rm -f "$INV"' EXIT
cat > "$INV" <<EOF
[nodes]
fixture ansible_ssh_host=${IP} ansible_ssh_user=root ansible_ssh_private_key_file=${KEY}
EOF

mkdir -p captures
rm -f "captures/${NAME}.jsonl"

ANSIBLE_COLLECTIONS_PATH="$(pwd)/galaxy" \
ANSIBLE_CALLBACK_PLUGINS=callback_plugins \
ANSIBLE_CALLBACKS_ENABLED=ruxel_capture \
ANSIBLE_HOST_KEY_CHECKING=False \
ANSIBLE_SSH_COMMON_ARGS="-o IdentitiesOnly=yes -o UserKnownHostsFile=${KEY}.known_hosts -o StrictHostKeyChecking=accept-new" \
RUXEL_CAPTURE_FILE="captures/${NAME}.jsonl" \
uv run ansible-playbook -i "$INV" "$PLAYBOOK"

echo "wrote captures/${NAME}.jsonl ($(wc -l < "captures/${NAME}.jsonl" | tr -d ' ') records)"

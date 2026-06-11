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

# Secretful playbooks: set RUXEL_DRY_SECRETS=1 so ansible resolves
# onepassword/pipe lookups to the same deterministic dry values ruxel's
# --dry-secrets produces (the fake onepassword is overlaid into galaxy's
# community.general; the fake pipe plugin is in lookup_plugins/). No real
# secret reaches the fixture, and ruxel↔ansible state stays byte-identical.
LOOKUP_ARGS=""
if [ "${RUXEL_DRY_SECRETS:-}" = "1" ]; then
  LOOKUP_ARGS="ANSIBLE_LOOKUP_PLUGINS=$(pwd)/lookup_plugins"
  # Self-heal the fake-onepassword overlay (galaxy/ is gitignored): copy
  # the dry-secret onepassword lookup over the real one so ansible resolves
  # to the same values ruxel does.
  GG="galaxy/ansible_collections/community/general/plugins/lookup"
  [ -d "$GG" ] && cp collections/ansible_collections/community/general/plugins/lookup/onepassword.py "$GG/onepassword.py"
fi

env $LOOKUP_ARGS \
ANSIBLE_COLLECTIONS_PATH="$(pwd)/galaxy" \
ANSIBLE_CALLBACK_PLUGINS=callback_plugins \
ANSIBLE_CALLBACKS_ENABLED=ruxel_capture \
ANSIBLE_HOST_KEY_CHECKING=False \
ANSIBLE_SSH_ARGS="-o ControlMaster=no -o ControlPath=none" \
ANSIBLE_SSH_COMMON_ARGS="-o IdentitiesOnly=yes -o UserKnownHostsFile=${KEY}.known_hosts -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=15 -o ServerAliveCountMax=4" \
RUXEL_CAPTURE_FILE="captures/${NAME}.jsonl" \
uv run ansible-playbook -i "$INV" "$PLAYBOOK"

echo "wrote captures/${NAME}.jsonl ($(wc -l < "captures/${NAME}.jsonl" | tr -d ' ') records)"

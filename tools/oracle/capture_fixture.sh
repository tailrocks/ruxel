#!/usr/bin/env bash
# Capture a real ansible-core 2.21 run of a workload playbook against a
# ruxel fixture VM (tools/fixtures/create.sh output), writing golden
# records. Targets come exclusively from the fixture scripts — never the
# production inventory (GOAL.md rule 2).
#
# Usage:
#   tools/oracle/capture_fixture.sh <fixture-name> <keyfile> <playbook-path> <capture-name> [group]
set -eu
CALLER_PWD="$PWD"
cd "$(dirname "$0")"

FIXTURE="${1:?provider fixture name}"
KEY="${2:?ssh keyfile}"
PLAYBOOK="${3:?playbook path}"
NAME="${4:?capture name}"
GROUP="${5:-nodes}"
[ "${PLAYBOOK#/}" != "$PLAYBOOK" ] || PLAYBOOK="$CALLER_PWD/$PLAYBOOK"

# shellcheck source=../fixtures/lib.sh
. ../fixtures/lib.sh
resolve_fixture "$FIXTURE"
require_fixture_key "$KEY"
IP="$FIXTURE_IP"

ROOT="$(cd ../.. && pwd)"
FIXTURE_ROOT="$ROOT/tools/fixture-project"
PLAYBOOK_REAL="$(realpath "$PLAYBOOK")"
case "$PLAYBOOK_REAL" in
  "$FIXTURE_ROOT"/*) PLAYBOOK="$PLAYBOOK_REAL" ;;
  *) echo "oracle: refusing non-fixture-project playbook: $PLAYBOOK" >&2; exit 1 ;;
esac

INV="$(mktemp)"
OVERLAY=""
trap 'rm -f "$INV"; [ -z "$OVERLAY" ] || rm -rf "$OVERLAY"' EXIT
cat > "$INV" <<EOF
[${GROUP}]
fixture ansible_ssh_host=${IP} ansible_ssh_user=root ansible_ssh_private_key_file=${KEY}
EOF

CAPTURE_DIR="${RUXEL_CAPTURE_DIR:-$(pwd)/captures}"
mkdir -p "$CAPTURE_DIR"
CAPTURE_FILE="$CAPTURE_DIR/${NAME}.jsonl"
rm -f "$CAPTURE_FILE"
touch "$CAPTURE_FILE"

# Secretful playbooks: set RUXEL_DRY_SECRETS=1 so ansible resolves
# onepassword/pipe lookups to the same deterministic dry values ruxel's
# --dry-secrets produces (the fake onepassword is overlaid into galaxy's
# community.general; the fake pipe plugin is in lookup_plugins/). No real
# secret reaches the fixture, and ruxel↔ansible state stays byte-identical.
LOOKUP_ARGS=""
COLLECTION_PATH="$(pwd)/galaxy"
if [ "${RUXEL_DRY_SECRETS:-}" = "1" ]; then
  LOOKUP_ARGS="ANSIBLE_LOOKUP_PLUGINS=$(pwd)/lookup_plugins"
  OVERLAY="$(mktemp -d)"
  cp -R galaxy/ansible_collections "$OVERLAY/"
  cp collections/ansible_collections/community/general/plugins/lookup/onepassword.py \
    "$OVERLAY/ansible_collections/community/general/plugins/lookup/onepassword.py"
  COLLECTION_PATH="$OVERLAY:$(pwd)/galaxy"
fi

ANSIBLE_ARGS=()
[ "${RUXEL_CAPTURE_CHECK:-0}" = "1" ] && ANSIBLE_ARGS+=(--check)
[ "${RUXEL_CAPTURE_DIFF:-0}" = "1" ] && ANSIBLE_ARGS+=(--diff)

set +e
env $LOOKUP_ARGS \
ANSIBLE_COLLECTIONS_PATH="$COLLECTION_PATH" \
ANSIBLE_CALLBACK_PLUGINS=callback_plugins \
ANSIBLE_CALLBACKS_ENABLED=ruxel_capture \
ANSIBLE_GATHERING=explicit \
ANSIBLE_HOST_KEY_CHECKING=False \
ANSIBLE_SSH_ARGS="-o ControlMaster=no -o ControlPath=none" \
ANSIBLE_SSH_COMMON_ARGS="-o IdentitiesOnly=yes -o UserKnownHostsFile=${KEY}.known_hosts -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=15 -o ServerAliveCountMax=4" \
RUXEL_CAPTURE_FILE="$CAPTURE_FILE" \
uv run ansible-playbook -i "$INV" "${ANSIBLE_ARGS[@]}" "$PLAYBOOK" \
  | tee "${RUXEL_CAPTURE_STDOUT_FILE:-/dev/null}"
ANSIBLE_STATUS=${PIPESTATUS[0]}
set -e

python3 normalize_capture.py "$CAPTURE_FILE"
[ -z "${RUXEL_CAPTURE_STATUS_FILE:-}" ] || printf '%s\n' "$ANSIBLE_STATUS" >"$RUXEL_CAPTURE_STATUS_FILE"

echo "wrote $CAPTURE_FILE ($(wc -l < "$CAPTURE_FILE" | tr -d ' ') records)"
[ "${RUXEL_CAPTURE_ALLOW_FAILURE:-0}" = 1 ] || exit "$ANSIBLE_STATUS"

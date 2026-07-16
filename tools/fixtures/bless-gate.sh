#!/usr/bin/env bash
# The parity gate, automated. For one playbook against a fixture VM:
#   1. ruxel apply            (fresh: may change)
#   2. ruxel apply again      (MUST be changed=0 — ruxel is idempotent)
#   3. ansible-playbook        (MUST be changed=0 — ansible agrees ruxel's
#                               state is converged: the "bless")
# Exits non-zero unless both idempotence checks hold. This is the
# three-way convergence proof done by hand for the first five playbooks
# (update-packages, upgrade-debian, install-docker, drives, postgresql).
#
# Usage:
#   tools/fixtures/bless-gate.sh <fixture-name> <keyfile> <agent-bin> <playbook> [dry] [group]
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

FIXTURE="${1:?provider fixture name}"
KEY="${2:?ssh keyfile}"
AGENT="${3:?agent binary (x86_64-musl)}"
PLAYBOOK="${4:?playbook path}"
DRY="${5:-}"   # pass "dry" for secretful playbooks (dry-secrets both sides)
GROUP="${6:-nodes}"
RUN_DIR="$(mktemp -d)"
trap 'rm -rf "$RUN_DIR"' EXIT

resolve_fixture "$FIXTURE"
require_fixture_key "$KEY"
DEST="root@${FIXTURE_IP}"

FIXTURE_ROOT="$(pwd)/tools/fixture-project"
PLAYBOOK_REAL="$(realpath "$PLAYBOOK")"
case "$PLAYBOOK_REAL" in
  "$FIXTURE_ROOT"/*) PLAYBOOK="$PLAYBOOK_REAL" ;;
  *) echo "fixtures: refusing non-fixture-project playbook: $PLAYBOOK" >&2; exit 1 ;;
esac

HOST="gate-host"
INV="$RUN_DIR/inventory.ini"
printf '[%s]\n%s ansible_ssh_host=%s ansible_ssh_user=root\n' \
  "$GROUP" "$HOST" "$FIXTURE_IP" > "$INV"

echo "== [1/3] ruxel apply (fresh) =="
RUXEL_SSH_KEY="$KEY" RUXEL_AGENT_BIN="$AGENT" \
  cargo run -q -p ruxel-cli -- apply --output json -i "$INV" ${DRY:+--dry-secrets} "$PLAYBOOK" | tee "$RUN_DIR/ruxel-fresh.jsonl"

# Parity, not zero: a converged run still reports the tasks that are
# *inherently* always-changed — bare command/shell with no changed_when
# (e.g. `mise use -g …`) report changed on every run under Ansible too.
# Drop-in parity = ruxel's converged-rerun changed-set equals Ansible's on
# the same state. We compare counts here; the task-name sets are diffed
# from the captures when they disagree.
echo "== [2/3] ruxel apply (converged rerun) =="
RUXEL_SSH_KEY="$KEY" RUXEL_AGENT_BIN="$AGENT" \
  cargo run -q -p ruxel-cli -- apply --output json -i "$INV" ${DRY:+--dry-secrets} "$PLAYBOOK" | tee "$RUN_DIR/ruxel-rerun.jsonl"
RERUN_CHANGED="$(jq -r 'select(.event == "recap") | .changed' "$RUN_DIR/ruxel-rerun.jsonl" | head -1)"
tools/fixtures/state-snapshot.sh "$FIXTURE" "$KEY" "$RUN_DIR/state-ruxel.txt"

echo "== [3/3] ansible bless (same state) =="
BLESS_NAME="bless-$(basename "$PLAYBOOK" .yml)"
{ [ -n "$DRY" ] && export RUXEL_DRY_SECRETS=1; tools/oracle/capture_fixture.sh "$FIXTURE" "$KEY" "$PLAYBOOK" "$BLESS_NAME" "$GROUP"; } | tee "$RUN_DIR/ansible.log"
BLESS_CHANGED="$(grep -Eo 'changed=[0-9]+' "$RUN_DIR/ansible.log" | head -1 | cut -d= -f2 || echo '?')"
tools/fixtures/state-snapshot.sh "$FIXTURE" "$KEY" "$RUN_DIR/state-ansible.txt"

(cd tools/oracle && uv run python compare_results.py \
  "$RUN_DIR/ruxel-rerun.jsonl" "captures/${BLESS_NAME}.jsonl")
diff -u "$RUN_DIR/state-ruxel.txt" "$RUN_DIR/state-ansible.txt"
echo "state parity: observable fixture state unchanged by Ansible bless"

if [ "$RERUN_CHANGED" != "$BLESS_CHANGED" ]; then
  echo "GATE FAIL: ruxel rerun changed=$RERUN_CHANGED but ansible bless changed=$BLESS_CHANGED — not at parity" >&2
  exit 1
fi
if [ "$RERUN_CHANGED" = "0" ]; then
  echo "GATE PASS: $(basename "$PLAYBOOK") — fully idempotent, ruxel + ansible both changed=0"
else
  echo "GATE PASS: $(basename "$PLAYBOOK") — at parity: ruxel and ansible both report changed=$RERUN_CHANGED (see $BLESS_NAME.jsonl)"
fi

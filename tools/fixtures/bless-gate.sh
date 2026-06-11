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
#   tools/fixtures/bless-gate.sh <dest> <keyfile> <agent-bin> <playbook> [inventory]
#
# <dest> must come from `hcloud server list` (GOAL.md rule 2). <inventory>
# defaults to a one-host ini built from <dest>; the playbook's `hosts:`
# must match (use `hosts: all`).
set -euo pipefail
cd "$(dirname "$0")/../.."

DEST="${1:?ssh destination (root@<fixture-ip>)}"
KEY="${2:?ssh keyfile}"
AGENT="${3:?agent binary (x86_64-musl)}"
PLAYBOOK="${4:?playbook path}"
INV="${5:-}"

IP="${DEST##*@}"
HOST="gate-host"

if [ -z "$INV" ]; then
  INV="$(mktemp)"
  trap 'rm -f "$INV"' EXIT
  printf '[nodes]\n%s ansible_ssh_host=%s ansible_ssh_user=root\n' "$HOST" "$IP" > "$INV"
fi

recap_changed() {
  # Extract the `changed=N` count from a ruxel/ansible PLAY RECAP line.
  grep -Eo 'changed=[0-9]+' | head -1 | cut -d= -f2
}

echo "== [1/3] ruxel apply (fresh) =="
RUXEL_SSH_KEY="$KEY" RUXEL_AGENT_BIN="$AGENT" \
  cargo run -q -p ruxel-cli -- apply -i "$INV" "$PLAYBOOK" | tee /tmp/gate-fresh.log

echo "== [2/3] ruxel apply (idempotence: expect changed=0) =="
RUXEL_SSH_KEY="$KEY" RUXEL_AGENT_BIN="$AGENT" \
  cargo run -q -p ruxel-cli -- apply -i "$INV" "$PLAYBOOK" | tee /tmp/gate-rerun.log
RERUN_CHANGED="$(recap_changed < /tmp/gate-rerun.log || echo '?')"
if [ "$RERUN_CHANGED" != "0" ]; then
  echo "GATE FAIL: ruxel rerun reported changed=$RERUN_CHANGED (not idempotent)" >&2
  exit 1
fi

echo "== [3/3] ansible bless (expect changed=0 on ruxel's state) =="
BLESS_NAME="bless-$(basename "$PLAYBOOK" .yml)"
tools/oracle/capture_fixture.sh "$IP" "$KEY" "$PLAYBOOK" "$BLESS_NAME" | tee /tmp/gate-bless.log
BLESS_CHANGED="$(grep -Eo 'changed=[0-9]+' /tmp/gate-bless.log | head -1 | cut -d= -f2 || echo '?')"
if [ "$BLESS_CHANGED" != "0" ]; then
  echo "GATE FAIL: ansible bless reported changed=$BLESS_CHANGED (ruxel's state not converged per ansible)" >&2
  exit 1
fi

echo "GATE PASS: $(basename "$PLAYBOOK") — ruxel idempotent + ansible-blessed (capture: $BLESS_NAME.jsonl)"

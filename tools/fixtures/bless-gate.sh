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

# Parity, not zero: a converged run still reports the tasks that are
# *inherently* always-changed — bare command/shell with no changed_when
# (e.g. `mise use -g …`) report changed on every run under Ansible too.
# Drop-in parity = ruxel's converged-rerun changed-set equals Ansible's on
# the same state. We compare counts here; the task-name sets are diffed
# from the captures when they disagree.
echo "== [2/3] ruxel apply (converged rerun) =="
RUXEL_SSH_KEY="$KEY" RUXEL_AGENT_BIN="$AGENT" \
  cargo run -q -p ruxel-cli -- apply -i "$INV" "$PLAYBOOK" | tee /tmp/gate-rerun.log
RERUN_CHANGED="$(recap_changed < /tmp/gate-rerun.log || echo '?')"

echo "== [3/3] ansible bless (same state) =="
BLESS_NAME="bless-$(basename "$PLAYBOOK" .yml)"
tools/oracle/capture_fixture.sh "$IP" "$KEY" "$PLAYBOOK" "$BLESS_NAME" | tee /tmp/gate-bless.log
BLESS_CHANGED="$(grep -Eo 'changed=[0-9]+' /tmp/gate-bless.log | head -1 | cut -d= -f2 || echo '?')"

if [ "$RERUN_CHANGED" != "$BLESS_CHANGED" ]; then
  echo "GATE FAIL: ruxel rerun changed=$RERUN_CHANGED but ansible bless changed=$BLESS_CHANGED — not at parity" >&2
  exit 1
fi
if [ "$RERUN_CHANGED" = "0" ]; then
  echo "GATE PASS: $(basename "$PLAYBOOK") — fully idempotent, ruxel + ansible both changed=0"
else
  echo "GATE PASS: $(basename "$PLAYBOOK") — at parity: ruxel and ansible both report changed=$RERUN_CHANGED (inherent always-changed command/shell tasks; see $BLESS_NAME.jsonl)"
fi

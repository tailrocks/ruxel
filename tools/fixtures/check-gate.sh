#!/usr/bin/env bash
# Compare Ruxel and Ansible check+diff outcomes without mutating a fixture.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

FIXTURE="${1:?provider fixture name}"
KEY="${2:?fixture key}"
AGENT="${3:?agent binary}"
PLAYBOOK="$(realpath "${4:?fixture-project playbook}")"
resolve_fixture "$FIXTURE"
require_fixture_key "$KEY"

case "$PLAYBOOK" in
  "$(pwd)/tools/fixture-project/"*) ;;
  *) die "refusing non-fixture-project playbook: $PLAYBOOK" ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
inventory="$work/inventory.ini"
cat >"$inventory" <<EOF
[nodes]
fixture-check ansible_ssh_host=${FIXTURE_IP} ansible_ssh_user=root
EOF

tools/fixtures/state-snapshot.sh "$FIXTURE" "$KEY" "$work/before.txt"
target/debug/ruxel apply \
  -i "$inventory" --ssh-key "$KEY" --agent-bin "$AGENT" \
  --check --diff --dry-secrets --output json \
  "$PLAYBOOK" >"$work/ruxel.jsonl"
tools/fixtures/state-snapshot.sh "$FIXTURE" "$KEY" "$work/after-ruxel.txt"
diff -u "$work/before.txt" "$work/after-ruxel.txt"

stem="$(basename "$PLAYBOOK" .yml)"
capture="check-${stem}"
[ "$stem" = "check-semantics" ] && capture="$stem"
RUXEL_CAPTURE_CHECK=1 RUXEL_CAPTURE_DIFF=1 RUXEL_DRY_SECRETS=1 \
  tools/oracle/capture_fixture.sh "$FIXTURE" "$KEY" "$PLAYBOOK" "$capture"
tools/fixtures/state-snapshot.sh "$FIXTURE" "$KEY" "$work/after-ansible.txt"
diff -u "$work/before.txt" "$work/after-ansible.txt"
tools/oracle/compare_results.py \
  "$work/ruxel.jsonl" "tools/oracle/captures/${capture}.jsonl"

echo "CHECK GATE PASS: $(basename "$PLAYBOOK") — result parity; zero state mutation"

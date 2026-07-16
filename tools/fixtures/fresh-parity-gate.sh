#!/usr/bin/env bash
# Full Ansible/Ruxel parity gate on two equivalent provider-verified fixtures.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

RUXEL_FIXTURE="${1:?Ruxel fixture name}"
RUXEL_KEY="${2:?Ruxel fixture key}"
ANSIBLE_FIXTURE="${3:?Ansible fixture name}"
ANSIBLE_KEY="${4:?Ansible fixture key}"
AGENT="$(realpath "${5:?agent binary}")"
PLAYBOOK="$(realpath "${6:?fixture playbook}")"
DRY="${7:-}"
GROUP="${8:-nodes}"

case "$PLAYBOOK" in
  "$(pwd)/tools/fixture-project/"*) ;;
  *) die "refusing non-fixture-project playbook: $PLAYBOOK" ;;
esac

resolve_fixture "$RUXEL_FIXTURE"
require_fixture_key "$RUXEL_KEY"
RUXEL_IP="$FIXTURE_IP"
resolve_fixture "$ANSIBLE_FIXTURE"
require_fixture_key "$ANSIBLE_KEY"
ANSIBLE_IP="$FIXTURE_IP"
[ "$RUXEL_FIXTURE" != "$ANSIBLE_FIXTURE" ] || die "fresh parity requires distinct fixtures"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
inventory="$work/ruxel.ini"
printf '[%s]\nfixture ansible_ssh_host=%s ansible_ssh_user=root\n' \
  "$GROUP" "$RUXEL_IP" >"$inventory"
stem="$(basename "$PLAYBOOK" .yml)"
ruxel=(target/debug/ruxel apply -i "$inventory" --ssh-key "$RUXEL_KEY"
  --agent-bin "$AGENT" --output json)
[ -z "$DRY" ] || ruxel+=(--dry-secrets)

snapshot() { tools/fixtures/state-snapshot.sh "$1" "$2" "$3"; }
capture() {
  local name="$1"
  shift
  RUXEL_DRY_SECRETS="$([ -z "$DRY" ] && echo 0 || echo 1)" \
    "$@" tools/oracle/capture_fixture.sh \
    "$ANSIBLE_FIXTURE" "$ANSIBLE_KEY" "$PLAYBOOK" "$name" "$GROUP"
}

snapshot "$RUXEL_FIXTURE" "$RUXEL_KEY" "$work/base-ruxel"
snapshot "$ANSIBLE_FIXTURE" "$ANSIBLE_KEY" "$work/base-ansible"
diff -u "$work/base-ruxel" "$work/base-ansible"

"${ruxel[@]}" "$PLAYBOOK" >"$work/ruxel-fresh.jsonl"
capture "fresh-$stem"
tools/oracle/compare_results.py "$work/ruxel-fresh.jsonl" \
  "tools/oracle/captures/fresh-$stem.jsonl"
snapshot "$RUXEL_FIXTURE" "$RUXEL_KEY" "$work/fresh-ruxel"
snapshot "$ANSIBLE_FIXTURE" "$ANSIBLE_KEY" "$work/fresh-ansible"
diff -u "$work/fresh-ruxel" "$work/fresh-ansible"

"${ruxel[@]}" "$PLAYBOOK" >"$work/ruxel-converged.jsonl"
capture "converged-$stem"
tools/oracle/compare_results.py "$work/ruxel-converged.jsonl" \
  "tools/oracle/captures/converged-$stem.jsonl"
snapshot "$RUXEL_FIXTURE" "$RUXEL_KEY" "$work/converged-ruxel"
snapshot "$ANSIBLE_FIXTURE" "$ANSIBLE_KEY" "$work/converged-ansible"
diff -u "$work/converged-ruxel" "$work/converged-ansible"

"${ruxel[@]}" --check --diff "$PLAYBOOK" >"$work/ruxel-check.jsonl"
RUXEL_CAPTURE_CHECK=1 RUXEL_CAPTURE_DIFF=1 \
  capture "check-$stem" env RUXEL_CAPTURE_CHECK=1 RUXEL_CAPTURE_DIFF=1
tools/oracle/compare_results.py "$work/ruxel-check.jsonl" \
  "tools/oracle/captures/check-$stem.jsonl"
snapshot "$RUXEL_FIXTURE" "$RUXEL_KEY" "$work/check-ruxel"
snapshot "$ANSIBLE_FIXTURE" "$ANSIBLE_KEY" "$work/check-ansible"
diff -u "$work/converged-ruxel" "$work/check-ruxel"
diff -u "$work/converged-ansible" "$work/check-ansible"

echo "FRESH PARITY PASS: $stem — base, fresh, converged, check/diff, final state"

#!/usr/bin/env bash
# Full Ansible/Ruxel parity gate on two equivalent provider-verified fixtures.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

RUXEL_FIXTURE="${1:?Ruxel fixture name}"
RUXEL_KEY="${2:?Ruxel fixture key}"
ANSIBLE_FIXTURE="${3:?Ansible fixture name}"
ANSIBLE_KEY="${4:?Ansible fixture key}"
RUXEL="$(realpath "${5:?controller binary}")"
AGENT="$(realpath "${6:?agent binary}")"
PLAYBOOK="$(realpath "${7:?fixture playbook}")"
DRY="${8:-}"
GROUP="${9:-nodes}"

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
fixture_spec "$RUXEL_FIXTURE" >"$work/spec-ruxel.json"
fixture_spec "$ANSIBLE_FIXTURE" >"$work/spec-ansible.json"
diff -u "$work/spec-ruxel.json" "$work/spec-ansible.json"
printf '[%s]\nfixture ansible_ssh_host=%s ansible_ssh_user=root\n' \
  "$GROUP" "$RUXEL_IP" >"$inventory"
stem="$(basename "$PLAYBOOK" .yml)"
ruxel=("$RUXEL" apply -i "$inventory" --ssh-key "$RUXEL_KEY"
  --agent-bin "$AGENT" --output json)
[ -z "$DRY" ] || ruxel+=(--dry-secrets)

snapshot() { tools/fixtures/state-snapshot.sh "$1" "$2" "$3"; }
capture() {
  local name="$1"
  shift
  RUXEL_CAPTURE_DIR="$work/captures" \
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
  "$work/captures/fresh-$stem.jsonl"
snapshot "$RUXEL_FIXTURE" "$RUXEL_KEY" "$work/fresh-ruxel"
snapshot "$ANSIBLE_FIXTURE" "$ANSIBLE_KEY" "$work/fresh-ansible"
diff -u "$work/fresh-ruxel" "$work/fresh-ansible"

"${ruxel[@]}" "$PLAYBOOK" >"$work/ruxel-converged.jsonl"
capture "converged-$stem"
tools/oracle/compare_results.py "$work/ruxel-converged.jsonl" \
  "$work/captures/converged-$stem.jsonl"
snapshot "$RUXEL_FIXTURE" "$RUXEL_KEY" "$work/converged-ruxel"
snapshot "$ANSIBLE_FIXTURE" "$ANSIBLE_KEY" "$work/converged-ansible"
diff -u "$work/converged-ruxel" "$work/converged-ansible"

set +e
"${ruxel[@]}" --check --diff "$PLAYBOOK" >"$work/ruxel-check.jsonl"
ruxel_check_status=$?
set -e
RUXEL_CAPTURE_CHECK=1 RUXEL_CAPTURE_DIFF=1 \
  capture "check-$stem" env RUXEL_CAPTURE_CHECK=1 RUXEL_CAPTURE_DIFF=1 \
    RUXEL_CAPTURE_ALLOW_FAILURE=1 RUXEL_CAPTURE_STATUS_FILE="$work/ansible-check-status" \
    RUXEL_CAPTURE_STDOUT_FILE="$work/ansible-check.stdout"
ansible_check_status="$(cat "$work/ansible-check-status")"
if { [ "$ruxel_check_status" -eq 0 ] && [ "$ansible_check_status" -ne 0 ]; } || \
   { [ "$ruxel_check_status" -ne 0 ] && [ "$ansible_check_status" -eq 0 ]; }; then
  die "check-mode success/failure mismatch: ruxel=$ruxel_check_status ansible=$ansible_check_status"
fi
tools/oracle/compare_results.py --ignore-diffs "$work/ruxel-check.jsonl" \
  "$work/captures/check-$stem.jsonl"
tools/oracle/compare_diffs.py "$work/ruxel-check.jsonl" "$work/ansible-check.stdout"
snapshot "$RUXEL_FIXTURE" "$RUXEL_KEY" "$work/check-ruxel"
snapshot "$ANSIBLE_FIXTURE" "$ANSIBLE_KEY" "$work/check-ansible"
diff -u "$work/converged-ruxel" "$work/check-ruxel"
diff -u "$work/converged-ansible" "$work/check-ansible"

jq -n -S \
  --arg playbook "$stem" \
  --arg controller_sha256 "$(sha256sum "$RUXEL" | cut -d' ' -f1)" \
  --arg agent_sha256 "$(sha256sum "$AGENT" | cut -d' ' -f1)" \
  --arg fresh_capture_sha256 "$(sha256sum "$work/captures/fresh-$stem.jsonl" | cut -d' ' -f1)" \
  --arg converged_capture_sha256 "$(sha256sum "$work/captures/converged-$stem.jsonl" | cut -d' ' -f1)" \
  --arg check_capture_sha256 "$(sha256sum "$work/captures/check-$stem.jsonl" | cut -d' ' -f1)" \
  --slurpfile fixture_spec "$work/spec-ruxel.json" \
  '{schema: 1, playbook: $playbook, fixture_spec: $fixture_spec[0],
    binaries: {controller_sha256: $controller_sha256, agent_sha256: $agent_sha256},
    modes: {
      fresh: {capture_sha256: $fresh_capture_sha256, result_parity: true, state_parity: true},
      converged: {capture_sha256: $converged_capture_sha256, result_parity: true, state_parity: true},
      check_diff: {capture_sha256: $check_capture_sha256, result_parity: true, state_contract: true}
    }}' >"$work/pass.json"

if [ "${RUXEL_PROMOTE_CAPTURES:-0}" = 1 ]; then
  cp "$work/captures/"*.jsonl tools/oracle/captures/
  mkdir -p tools/oracle/parity
  cp "$work/pass.json" "tools/oracle/parity/$stem.json"
fi

echo "FRESH PARITY PASS: $stem — base, fresh, converged, check/diff, final state"

#!/usr/bin/env bash
# Disposable-provider SSH chaos acceptance gate. Never accepts a raw target.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

FIXTURE="${1:?provider fixture name}"
KEY="${2:?fixture key}"
RUXEL="$(realpath "${3:?ruxel binary}")"
AGENT="$(realpath "${4:?x86_64 Linux agent binary}")"
PLAYBOOK="$(realpath tools/fixture-project/chaos/chaos.yml)"
PAYLOAD="$(dirname "$PLAYBOOK")/large-payload.txt"
resolve_fixture "$FIXTURE"
require_fixture_key "$KEY"
[ -x "$RUXEL" ] || die "ruxel binary is not executable"
[ -x "$AGENT" ] || die "agent binary is not executable"

# Safety tests stop here. Provider identity and every local input have already
# been validated; importantly, no SSH command has run yet.
[ "${RUXEL_CHAOS_VALIDATE_ONLY:-0}" = 1 ] && exit 0

REAL_SSH="$(command -v ssh)"
case "$REAL_SSH" in
  "$(pwd)/tools/chaos/ssh") die "real ssh resolves to chaos proxy" ;;
esac
work="$(mktemp -d)"
runtime="$work/runtime"
mkdir -m 700 "$runtime"
controller_pids=()
cleanup() {
  for pid in "${controller_pids[@]}"; do
    kill -KILL "$pid" 2>/dev/null || true
  done
  pkill -f "$runtime/ruxel/" 2>/dev/null || true
  rm -f "$PAYLOAD"
  rm -rf "$work"
}
trap cleanup EXIT INT TERM
python3 tools/chaos/make_payload.py "$PAYLOAD"

DEST="root@${FIXTURE_IP}"
KNOWN_HOSTS="${KEY}.known_hosts"
inventory="$work/inventory.ini"
cat >"$inventory" <<EOF
[nodes]
fixture-chaos ansible_ssh_host=${FIXTURE_IP} ansible_ssh_user=root
EOF

safe_ssh() {
  "$REAL_SSH" -q -i "$KEY" -o IdentitiesOnly=yes \
    -o "UserKnownHostsFile=$KNOWN_HOSTS" -o StrictHostKeyChecking=accept-new \
    -o ConnectTimeout=10 "$DEST" -- "$@"
}

run_ruxel() {
  XDG_RUNTIME_DIR="$runtime" "$RUXEL" apply -i "$inventory" --ssh-key "$KEY" \
    --agent-bin "$AGENT" --output json --dry-secrets "$@" "$PLAYBOOK"
}

assert_converged() {
  local output="$1" tag="$2"
  python3 - "$output" "$tag" <<'PY'
import json, sys
records = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
recaps = [record for record in records if record.get("event") == "recap"]
assert len(recaps) == 1, (sys.argv[2], recaps)
assert recaps[0]["changed"] == 0 and recaps[0]["failed"] == 0, recaps[0]
PY
}

echo "Safety check: provider-verified disposable <fixture>"
run_ruxel >"$work/seed.jsonl"
run_ruxel >"$work/converged.jsonl"
assert_converged "$work/converged.jsonl" seed
tools/fixtures/state-snapshot.sh "$FIXTURE" "$KEY" "$work/baseline.state"

results="$work/results.jsonl"
cases=(upload-start partial-hello-ack large-plan large-task-result long-subprocess controlmaster-sigint)
for case_name in "${cases[@]}"; do
  rm -f "$runtime"/ruxel/* 2>/dev/null || true
  sentinel="$work/$case_name.sentinel"
  if [ "$case_name" = upload-start ]; then
    safe_ssh "find /var/lib/ruxel/agent -maxdepth 1 -type f -delete"
  fi
  if [ "$case_name" = large-task-result ] || [ "$case_name" = long-subprocess ]; then
    # Defeat both the creates guard and its ledger fingerprint so the fault
    # reaches the real large-result/long-subprocess boundary.
    safe_ssh "rm -f /tmp/ruxel-fixture-chaos/$case_name"
  fi

  started_ms="$(python3 -c 'import time; print(time.monotonic_ns() // 1000000)')"
  (
    trap - INT TERM
    export XDG_RUNTIME_DIR="$runtime"
    export PATH="$(pwd)/tools/chaos:$PATH"
    export RUXEL_CHAOS_REAL_SSH="$REAL_SSH"
    export RUXEL_CHAOS_CASE="$case_name"
    export RUXEL_CHAOS_SENTINEL="$sentinel"
    exec "$RUXEL" apply -i "$inventory" --ssh-key "$KEY" --agent-bin "$AGENT" \
      --output json --dry-secrets --tags "$case_name" "$PLAYBOOK"
  ) >"$work/$case_name.interrupted.jsonl" \
    2>"$work/$case_name.interrupted.stderr" &
  pid=$!
  controller_pids+=("$pid")

  observed=0
  for _ in $(seq 1 300); do
    if [ -f "$sentinel" ]; then observed=1; break; fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.05
  done
  [ "$observed" -eq 1 ] || die "$case_name injection boundary was not observed"
  if [ "$case_name" = long-subprocess ] || [ "$case_name" = controlmaster-sigint ] || [ "$case_name" = upload-start ]; then
    kill -INT "$pid" 2>/dev/null || true
  fi
  set +e
  wait "$pid"
  interrupted_status=$?
  set -e
  [ "$interrupted_status" -ne 0 ] || die "$case_name unexpectedly succeeded"

  flock_free=0
  for _ in $(seq 1 240); do
    if safe_ssh "flock -n /var/lib/ruxel/agent.lock true"; then flock_free=1; break; fi
    sleep 0.25
  done
  [ "$flock_free" -eq 1 ] || die "$case_name agent flock remained held"

  run_ruxel --tags "$case_name" >"$work/$case_name.recovery.jsonl"
  assert_converged "$work/$case_name.recovery.jsonl" "$case_name"
  tools/fixtures/state-snapshot.sh "$FIXTURE" "$KEY" "$work/$case_name.state"
  cmp "$work/baseline.state" "$work/$case_name.state" \
    || die "$case_name recovery state differs from seeded state"
  safe_ssh "! pgrep -f '^/var/lib/ruxel/agent/' >/dev/null"
  safe_ssh "! find /var/lib/ruxel/agent -maxdepth 1 -name '*.tmp-*' -print -quit | grep -q ."
  ! pgrep -f "$runtime/ruxel/" >/dev/null \
    || die "$case_name leaked a local SSH/ControlMaster process"
  [ -z "$(find "$runtime/ruxel" -type s -o -type f 2>/dev/null || true)" ] \
    || die "$case_name leaked a ControlMaster socket"
  ended_ms="$(python3 -c 'import time; print(time.monotonic_ns() // 1000000)')"
  elapsed_ms=$((ended_ms - started_ms))
  [ "$elapsed_ms" -le 120000 ] || die "$case_name recovery exceeded 120 seconds"
  printf '{"case":"%s","injection_sentinel":true,"interrupted_status":%s,"reconnect":true,"flock_free":true,"converged":true,"converged_changed":0,"converged_failed":0,"state_equal":true,"no_process_leak":true,"no_socket_leak":true,"no_temp_leak":true,"recovery_elapsed_ms":%s,"recovery_timeout_ms":120000}\n' \
    "$case_name" "$interrupted_status" "$elapsed_ms" >>"$results"
done

mkdir -p tools/chaos/artifacts
python3 - "$results" >tools/chaos/artifacts/manifest.json <<'PY'
import json, sys
print(json.dumps({
    "schema_version": 1,
    "target": "<fixture>",
    "cases": [json.loads(line) for line in open(sys.argv[1])],
}, indent=2, sort_keys=True))
PY
python3 tools/chaos/verify.py
echo "CHAOS PASS: six deterministic SSH boundaries recovered leak-free"

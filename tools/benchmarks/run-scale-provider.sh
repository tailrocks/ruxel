#!/usr/bin/env bash
# Run the fixed synthetic 65-task/52-lookup benchmark on disposable provider twins.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

CONTROLLER="${1:-}"
AGENT="${2:-}"
REPS="${3:-3}"
RESULTS_ROOT="${4:-docs/benchmarks/results}"
case "$REPS" in ''|*[!0-9]*) echo "scale benchmark: repetitions must be an integer >=3" >&2; exit 2;; esac
[ "$REPS" -ge 3 ] || { echo "scale benchmark: repetitions must be >=3" >&2; exit 2; }
[ -x "$CONTROLLER" ] || { echo "scale benchmark: controller is not executable" >&2; exit 2; }
[ -x "$AGENT" ] || { echo "scale benchmark: agent is not executable" >&2; exit 2; }
CONTROLLER="$(realpath "$CONTROLLER")"
AGENT="$(realpath "$AGENT")"
PLAYBOOK="$(realpath tools/benchmarks/fixtures/scale-65x52.yml)"
python3 tools/benchmarks/validate_scale.py "$PLAYBOOK"

final_dir="$RESULTS_ROOT/scale-65x52"
[ ! -e "$final_dir" ] || die "refusing to overwrite benchmark evidence: $final_dir"
stage="$(mktemp -d "${TMPDIR:-/tmp}/ruxel-scale-benchmark.XXXXXX")"
case_dir="$stage/scale-65x52"
mkdir -p "$case_dir/logs" "$case_dir/correctness"
mkdir -p "$stage/collection-overlay"
cp -R tools/oracle/galaxy/ansible_collections "$stage/collection-overlay/"
cp tools/oracle/collections/ansible_collections/community/general/plugins/lookup/onepassword.py \
  "$stage/collection-overlay/ansible_collections/community/general/plugins/lookup/onepassword.py"
active=()

cleanup_fixture() {
  local fixture="$1" suffix="${1#ruxel-fixture-}"
  if hcloud server list -l ruxel=fixture -o noheader -o columns=name 2>/dev/null | grep -Fxq "$fixture"; then
    hcloud server delete "$fixture" >/dev/null 2>&1 || true
  fi
  hcloud ssh-key delete "$(session_key_name "$suffix")" >/dev/null 2>&1 || true
  rm -f "${TMPDIR:-/tmp}/${fixture}-ssh" "${TMPDIR:-/tmp}/${fixture}-ssh.pub" \
    "${TMPDIR:-/tmp}/${fixture}-ssh.known_hosts"
}
cleanup() {
  local fixture
  for fixture in "${active[@]}"; do cleanup_fixture "$fixture"; done
  rm -rf "$stage"
}
trap cleanup EXIT INT TERM

create_twin() {
  local suffix="$1" output
  active+=("ruxel-fixture-$suffix")
  output="$(tools/fixtures/create.sh "$suffix")"
  CREATED_NAME="$(sed -n 's/^RUXEL_FIXTURE_NAME=//p' <<<"$output" | tail -1)"
  CREATED_KEY="$(sed -n 's/^RUXEL_FIXTURE_KEY=//p' <<<"$output" | tail -1)"
  [ -n "$CREATED_NAME" ] && [ -n "$CREATED_KEY" ] || die "fixture creation output incomplete"
}

make_inventory() {
  local fixture="$1" key="$2" destination="$3"
  resolve_fixture "$fixture"; require_fixture_key "$key"
  printf '[benchmark]\nfixture ansible_ssh_host=%s ansible_ssh_user=root\n' "$FIXTURE_IP" >"$destination"
}

append_sample() {
  local executor="$1" repetition="$2" order="$3" elapsed="$4" stdout="$5" stderr="$6"
  python3 tools/benchmarks/artifact.py "$case_dir" "$executor" "$repetition" "$order" \
    "$elapsed" "$stdout" "$stderr"
}

create_twin "scale-$PPID-r"; RFIXTURE="$CREATED_NAME"; RKEY="$CREATED_KEY"
create_twin "scale-$PPID-a"; AFIXTURE="$CREATED_NAME"; AKEY="$CREATED_KEY"
fixture_spec "$RFIXTURE" >"$stage/spec-ruxel.json"
fixture_spec "$AFIXTURE" >"$stage/spec-ansible.json"
diff -u "$stage/spec-ruxel.json" "$stage/spec-ansible.json"
make_inventory "$RFIXTURE" "$RKEY" "$stage/ruxel.ini"
make_inventory "$AFIXTURE" "$AKEY" "$stage/ansible.ini"
resolve_fixture "$RFIXTURE"; RIP="$FIXTURE_IP"
resolve_fixture "$AFIXTURE"; AIP="$FIXTURE_IP"

for repetition in $(seq 1 "$REPS"); do
  capture="$stage/ansible-$repetition.jsonl"
  rstdout="$case_dir/logs/ruxel-$repetition.stdout"; rstderr="$case_dir/logs/ruxel-$repetition.stderr"
  astdout="$case_dir/logs/ansible-$repetition.stdout"; astderr="$case_dir/logs/ansible-$repetition.stderr"
  relapsed="$stage/ruxel.elapsed"; rstatus="$stage/ruxel.status"
  aelapsed="$stage/ansible.elapsed"; astatus="$stage/ansible.status"
  rcmd=("$CONTROLLER" apply -i "$stage/ruxel.ini" --ssh-key "$RKEY" --agent-bin "$AGENT" --output json --dry-secrets "$PLAYBOOK")
  acmd=(env
    "ANSIBLE_COLLECTIONS_PATH=$stage/collection-overlay:$(pwd)/tools/oracle/galaxy"
    "ANSIBLE_LOOKUP_PLUGINS=$(pwd)/tools/oracle/lookup_plugins"
    "ANSIBLE_CALLBACK_PLUGINS=$(pwd)/tools/oracle/callback_plugins"
    ANSIBLE_CALLBACKS_ENABLED=ruxel_capture ANSIBLE_GATHERING=explicit ANSIBLE_HOST_KEY_CHECKING=False
    ANSIBLE_SSH_RETRIES=3 "ANSIBLE_SSH_ARGS=-o ControlMaster=no -o ControlPath=none"
    "ANSIBLE_SSH_COMMON_ARGS=-o IdentitiesOnly=yes -o UserKnownHostsFile=${AKEY}.known_hosts -o StrictHostKeyChecking=accept-new"
    "RUXEL_CAPTURE_FILE=$capture" uv run ansible-playbook -i "$stage/ansible.ini" "$PLAYBOOK")
  run_ruxel() { python3 tools/benchmarks/run_timed.py "$relapsed" "$rstatus" "$rstdout" "$rstderr" -- "${rcmd[@]}"; }
  run_ansible() { python3 tools/benchmarks/run_timed.py "$aelapsed" "$astatus" "$astdout" "$astderr" -- "${acmd[@]}"; }
  if [ $((repetition % 2)) -eq 1 ]; then run_ansible; run_ruxel; aorder=1; rorder=2; else run_ruxel; run_ansible; rorder=1; aorder=2; fi
  [ "$(cat "$rstatus")" -eq 0 ] && [ "$(cat "$astatus")" -eq 0 ] || die "timed scale command failed"
  python3 tools/oracle/normalize_capture.py "$capture"
  tools/oracle/compare_results.py "$rstdout" "$capture"
  tools/fixtures/state-snapshot.sh "$RFIXTURE" "$RKEY" "$stage/state-ruxel"
  tools/fixtures/state-snapshot.sh "$AFIXTURE" "$AKEY" "$stage/state-ansible"
  diff -u "$stage/state-ruxel" "$stage/state-ansible" >"$case_dir/correctness/state-$repetition.diff"
  append_sample ruxel "$repetition" "$rorder" "$(cat "$relapsed")" "$rstdout" "$rstderr"
  append_sample ansible "$repetition" "$aorder" "$(cat "$aelapsed")" "$astdout" "$astderr"
done

tools/fixtures/destroy.sh "$RFIXTURE"; tools/fixtures/destroy.sh "$AFIXTURE"; active=()
for fixture in "$RFIXTURE" "$AFIXTURE"; do
  ! hcloud server list -l ruxel=fixture -o noheader -o columns=name | grep -Fxq "$fixture" \
    || die "fixture still exists after destroy: $fixture"
done

controller_sha="$(sha256sum "$CONTROLLER" | cut -d' ' -f1)"; agent_sha="$(sha256sum "$AGENT" | cut -d' ' -f1)"
fixture_sha="$(sha256sum "$PLAYBOOK" | cut -d' ' -f1)"
crate_version="$(cargo metadata --no-deps --format-version 1 | jq -er '.packages[] | select(.name=="ruxel-cli") | .version')"
jq -n -S --arg fixture_sha "$fixture_sha" --arg controller_sha "$controller_sha" --arg agent_sha "$agent_sha" \
  --arg ansible "$(tools/oracle/.venv/bin/ansible-playbook --version | head -1)" --arg ruxel "$crate_version" \
  --arg rustc "$(rustc --version)" --arg os "$(uname -srm)" --arg kernel "$(uname -r)" --argjson repetitions "$REPS" \
  --slurpfile specification "$stage/spec-ruxel.json" \
  '{schema:1,case:"scale-65x52",playbook:"tools/benchmarks/fixtures/scale-65x52.yml",fixture_source_sha256:$fixture_sha,
    binaries:{controller_sha256:$controller_sha,agent_sha256:$agent_sha},
    versions:{ansible:$ansible,ruxel:$ruxel,agent:$ruxel,rustc:$rustc,os:$os,kernel:$kernel},
    fixture:{kind:"disposable-provider-twin",specification:$specification[0]},repetitions:$repetitions,
    scale_gate:{task_count:65,synthetic_lookup_count:52,dry_secrets:true,ruxel_median_limit_ns:5000000000},
    correctness:{fixture_identity_verified:true,result_parity:true,diff_parity:true,state_parity:true,resources_reaped:true}}' \
  >"$case_dir/manifest.json"
python3 tools/benchmarks/summarize.py "$case_dir"
python3 tools/benchmarks/verify.py --case "$case_dir" scale-65x52
mkdir -p "$RESULTS_ROOT"; mv "$case_dir" "$final_dir"
echo "SCALE BENCHMARK PASS: $REPS repetitions per executor; Ruxel median <5s"

#!/usr/bin/env bash
# Correctness-gated benchmarks on exactly two provider-verified disposable twins.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

CASE="${1:-}"
CONTROLLER="${2:-}"
AGENT="${3:-}"
REPS="${4:-3}"
RESULTS_ROOT="${5:-docs/benchmarks/results}"

case "$CASE" in
  fresh) playbook=benchmarks/files.yml; parity_playbook=files-content.yml; group=all; scenario=fresh; prepare=; dry= ;;
  converged) playbook=benchmarks/files.yml; parity_playbook=files-content.yml; group=all; scenario=converged; prepare=; dry= ;;
  one-task-drift) playbook=benchmarks/files.yml; parity_playbook=files-content.yml; group=all; scenario=one-task-drift; prepare=; dry= ;;
  check-diff) playbook=benchmarks/files.yml; parity_playbook=files-content.yml; group=all; scenario=check-diff; prepare=; dry= ;;
  secret) playbook=performance-snapshots.yml; group=performance; scenario=converged; prepare=postgresql; dry=1 ;;
  storage) playbook=storage-ext4.yml; group=storage; scenario=converged; prepare=storage-ext4; dry= ;;
  postgresql) playbook=postgresql-ownership.yml; group=postgresql; scenario=converged; prepare=postgresql; dry= ;;
  simulated-rtt) playbook=benchmarks/files.yml; parity_playbook=files-content.yml; group=all; scenario=simulated-rtt; prepare=; dry= ;;
  *) echo "benchmark: case must be one of fresh, converged, one-task-drift, check-diff, secret, storage, postgresql, simulated-rtt" >&2; exit 2 ;;
esac
parity_playbook="${parity_playbook:-$playbook}"
case "$REPS" in ''|*[!0-9]*) echo "benchmark: repetitions must be an integer >=3" >&2; exit 2;; esac
[ "$REPS" -ge 3 ] || { echo "benchmark: repetitions must be >=3" >&2; exit 2; }
[ -x "$CONTROLLER" ] || { echo "benchmark: controller is not executable" >&2; exit 2; }
[ -x "$AGENT" ] || { echo "benchmark: agent is not executable" >&2; exit 2; }
CONTROLLER="$(realpath "$CONTROLLER")"
AGENT="$(realpath "$AGENT")"
PLAYBOOK="$(realpath "tools/fixture-project/$playbook")"
PARITY="tools/oracle/parity/${parity_playbook%.yml}.json"
[ -f "$PARITY" ] || die "missing correctness manifest: $PARITY"
controller_sha="$(sha256sum "$CONTROLLER" | cut -d' ' -f1)"
agent_sha="$(sha256sum "$AGENT" | cut -d' ' -f1)"
jq -e '.modes.fresh.result_parity and .modes.fresh.state_parity and
  .modes.converged.result_parity and .modes.converged.state_parity and
  .modes.check_diff.result_parity and .modes.check_diff.state_contract' "$PARITY" >/dev/null \
  || die "parity manifest does not prove every mode: $PARITY"
jq -e --arg controller "$controller_sha" --arg agent "$agent_sha" \
  '.binaries.controller_sha256 == $controller and .binaries.agent_sha256 == $agent' "$PARITY" >/dev/null \
  || die "parity evidence was produced by different binaries: $PARITY"

final_dir="$RESULTS_ROOT/$CASE"
[ ! -e "$final_dir" ] || die "refusing to overwrite benchmark evidence: $final_dir"
stage="$(mktemp -d "${TMPDIR:-/tmp}/ruxel-benchmark-${CASE}.XXXXXX")"
case_dir="$stage/$CASE"
mkdir -p "$case_dir/logs" "$case_dir/correctness"
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

destroy_twins() {
  local fixture
  for fixture in "${active[@]}"; do tools/fixtures/destroy.sh "$fixture"; done
  active=()
}

prepare_fixture() {
  local fixture="$1" key="$2"
  case "$prepare" in
    '') ;;
    postgresql) tools/fixtures/prepare-postgresql.sh "$fixture" "$key" ;;
    storage-ext4) tools/fixtures/prepare-storage.sh "$fixture" "$key" storage-ext4 ;;
    *) die "internal invalid preparation" ;;
  esac
}

remote_script() {
  local fixture="$1" key="$2" script="$3"
  resolve_fixture "$fixture"
  require_fixture_key "$key"
  local attempt
  for attempt in 1 2 3 4; do
    if ssh -i "$key" -o IdentitiesOnly=yes -o ConnectTimeout=15 \
      -o "UserKnownHostsFile=${key}.known_hosts" -o StrictHostKeyChecking=accept-new \
      "root@${FIXTURE_IP}" bash -se <<<"$script"; then
      return 0
    fi
    [ "$attempt" -eq 4 ] || sleep "$attempt"
  done
  die "remote fixture command failed after four attempts"
}

snapshot() { tools/fixtures/state-snapshot.sh "$1" "$2" "$3"; }

make_inventory() {
  local fixture="$1" key="$2" destination="$3"
  resolve_fixture "$fixture"
  require_fixture_key "$key"
  printf '[%s]\nfixture ansible_ssh_host=%s ansible_ssh_user=root\n' "$group" "$FIXTURE_IP" >"$destination"
}

sanitize_log() {
  local source="$1" destination="$2" ip="$3" key="$4"
  python3 - "$source" "$destination" "$ip" "$key" "$(pwd)" <<'PY'
import pathlib, re, sys
source, destination, ip, key, cwd = sys.argv[1:]
text = pathlib.Path(source).read_text(encoding="utf-8", errors="replace")
for value, replacement in ((ip, "<fixture-ip>"), (key, "<fixture-key>"), (cwd, "<repository>")):
    text = text.replace(value, replacement)
text = re.sub(r"/Users/[^/\s]+", "<controller-home>", text)
text = re.sub(r"/home/[^/\s]+", "<controller-home>", text)
pathlib.Path(destination).write_text(text, encoding="utf-8")
PY
}

append_sample() {
  local executor="$1" repetition="$2" order="$3" elapsed="$4" stdout="$5" stderr="$6"
  local stdout_sha stderr_sha
  stdout_sha="$(sha256sum "$stdout" | cut -d' ' -f1)"
  stderr_sha="$(sha256sum "$stderr" | cut -d' ' -f1)"
  jq -cn --arg executor "$executor" --argjson repetition "$repetition" \
    --argjson order "$order" --argjson elapsed_ns "$elapsed" \
    --arg stdout_path "${stdout#"$case_dir/"}" --arg stdout_sha "$stdout_sha" \
    --arg stderr_path "${stderr#"$case_dir/"}" --arg stderr_sha "$stderr_sha" \
    '{executor:$executor,repetition:$repetition,execution_order:$order,accepted:true,elapsed_ns:$elapsed_ns,
      stdout:{path:$stdout_path,sha256:$stdout_sha},stderr:{path:$stderr_path,sha256:$stderr_sha}}' \
    >>"$case_dir/samples.jsonl"
}

run_pair() {
  local repetition="$1" rfixture="$2" rkey="$3" afixture="$4" akey="$5"
  local rinv="$stage/ruxel.ini" name="${CASE}-${repetition}"
  local rraw="$stage/ruxel.stdout" reraw="$stage/ruxel.stderr"
  local araw="$stage/ansible.stdout" aeraw="$stage/ansible.stderr"
  local relapsed="$stage/ruxel.elapsed" rstatus="$stage/ruxel.status"
  local aelapsed="$stage/ansible.elapsed" astatus="$stage/ansible.status"
  local capture_dir="$stage/captures-$repetition"
  local aorder rorder
  make_inventory "$rfixture" "$rkey" "$rinv"
  resolve_fixture "$rfixture"; local rip="$FIXTURE_IP"
  resolve_fixture "$afixture"; local aip="$FIXTURE_IP"
  local rcmd=("$CONTROLLER" apply -i "$rinv" --ssh-key "$rkey" --agent-bin "$AGENT" --output json)
  [ -z "$dry" ] || rcmd+=(--dry-secrets)
  if [ "$scenario" = check-diff ]; then rcmd+=(--check --diff); fi
  rcmd+=("$PLAYBOOK")
  local acmd=(env "RUXEL_CAPTURE_DIR=$capture_dir" "RUXEL_DRY_SECRETS=$([ -n "$dry" ] && echo 1 || echo 0)")
  acmd+=("RUXEL_CAPTURE_BENCH_ELAPSED=$aelapsed" "RUXEL_CAPTURE_BENCH_STATUS=$astatus"
    "RUXEL_CAPTURE_BENCH_STDOUT=$araw" "RUXEL_CAPTURE_BENCH_STDERR=$aeraw")
  if [ "$scenario" = check-diff ]; then
    acmd+=(RUXEL_CAPTURE_CHECK=1 RUXEL_CAPTURE_DIFF=1)
  fi
  acmd+=(tools/oracle/capture_fixture.sh "$afixture" "$akey" "$PLAYBOOK" "$name" "$group")

  run_one() {
    local executor="$1"
    if [ "$executor" = ansible ]; then
      "${acmd[@]}" >"$stage/ansible-helper.stdout" 2>"$stage/ansible-helper.stderr"
    else
      python3 tools/benchmarks/run_timed.py "$relapsed" "$rstatus" "$rraw" "$reraw" -- "${rcmd[@]}"
    fi
  }
  if [ $((repetition % 2)) -eq 1 ]; then run_one ansible; run_one ruxel; else run_one ruxel; run_one ansible; fi
  if [ "$(cat "$rstatus")" -ne 0 ] || [ "$(cat "$astatus")" -ne 0 ]; then
    echo "ruxel status=$(cat "$rstatus") ansible status=$(cat "$astatus")" >&2
    tail -40 "$reraw" >&2 || true
    tail -40 "$aeraw" >&2 || true
    die "timed command failed"
  fi
  if [ "$scenario" = check-diff ]; then
    tools/oracle/compare_results.py --ignore-diffs "$rraw" "$capture_dir/$name.jsonl"
    tools/oracle/compare_diffs.py "$rraw" "$araw"
  else
    tools/oracle/compare_results.py "$rraw" "$capture_dir/$name.jsonl"
  fi
  if [ "$scenario" = one-task-drift ] || [ "$scenario" = check-diff ]; then
    python3 - "$rraw" "$scenario" <<'PY'
import json, sys
records = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
changed = [record.get("task", "").split(" : ")[-1] for record in records
           if record.get("event") == "task" and record.get("status") == "changed"]
expected = ("Exercise command mapping and creates" if sys.argv[2] == "one-task-drift"
            else "Exercise managed block")
assert changed == [expected], f"expected exactly one drifted task {expected!r}, got {changed!r}"
PY
  elif [ "$scenario" = converged ] && [ "$playbook" = benchmarks/files.yml ]; then
    python3 - "$rraw" <<'PY'
import json, sys
changed = [record.get("task") for record in map(json.loads, open(sys.argv[1]))
           if record.get("event") == "task" and record.get("status") == "changed"]
assert not changed, f"converged benchmark changed tasks: {changed!r}"
PY
  fi
  snapshot "$rfixture" "$rkey" "$stage/state-ruxel"
  snapshot "$afixture" "$akey" "$stage/state-ansible"
  diff -u "$stage/state-ruxel" "$stage/state-ansible" >"$case_dir/correctness/state-$repetition.diff"
  if [ "$scenario" = check-diff ]; then
    diff -u "$stage/pre-ruxel" "$stage/state-ruxel" >"$case_dir/correctness/ruxel-check-$repetition.diff"
    diff -u "$stage/pre-ansible" "$stage/state-ansible" >"$case_dir/correctness/ansible-check-$repetition.diff"
  fi
  local rout="$case_dir/logs/ruxel-$repetition.stdout" rerr="$case_dir/logs/ruxel-$repetition.stderr"
  local aout="$case_dir/logs/ansible-$repetition.stdout" aerr="$case_dir/logs/ansible-$repetition.stderr"
  sanitize_log "$rraw" "$rout" "$rip" "$rkey"; sanitize_log "$reraw" "$rerr" "$rip" "$rkey"
  sanitize_log "$araw" "$aout" "$aip" "$akey"; sanitize_log "$aeraw" "$aerr" "$aip" "$akey"
  if [ $((repetition % 2)) -eq 1 ]; then aorder=1; rorder=2; else rorder=1; aorder=2; fi
  append_sample ruxel "$repetition" "$rorder" "$(cat "$relapsed")" "$rout" "$rerr"
  append_sample ansible "$repetition" "$aorder" "$(cat "$aelapsed")" "$aout" "$aerr"
}

create_and_prepare_twins() {
  local token="$1"
  create_twin "bench-${CASE}-${token}-r"; RFIXTURE="$CREATED_NAME"; RKEY="$CREATED_KEY"
  create_twin "bench-${CASE}-${token}-a"; AFIXTURE="$CREATED_NAME"; AKEY="$CREATED_KEY"
  prepare_fixture "$RFIXTURE" "$RKEY"; prepare_fixture "$AFIXTURE" "$AKEY"
  fixture_spec "$RFIXTURE" >"$stage/spec-ruxel.json"
  fixture_spec "$AFIXTURE" >"$stage/spec-ansible.json"
  diff -u "$stage/spec-ruxel.json" "$stage/spec-ansible.json"
  snapshot "$RFIXTURE" "$RKEY" "$stage/base-ruxel"
  snapshot "$AFIXTURE" "$AKEY" "$stage/base-ansible"
  diff -u "$stage/base-ruxel" "$stage/base-ansible"
}

untimed_converge() {
  local rinv="$stage/setup-ruxel.ini"
  make_inventory "$RFIXTURE" "$RKEY" "$rinv"
  local args=("$CONTROLLER" apply -i "$rinv" --ssh-key "$RKEY" --agent-bin "$AGENT" --output json)
  [ -z "$dry" ] || args+=(--dry-secrets)
  "${args[@]}" "$PLAYBOOK" >"$stage/setup-ruxel.jsonl"
  RUXEL_CAPTURE_DIR="$stage/setup-capture" RUXEL_DRY_SECRETS="$([ -n "$dry" ] && echo 1 || echo 0)" \
    tools/oracle/capture_fixture.sh "$AFIXTURE" "$AKEY" "$PLAYBOOK" setup "$group" >"$stage/setup-ansible.stdout"
  tools/oracle/compare_results.py "$stage/setup-ruxel.jsonl" "$stage/setup-capture/setup.jsonl"
  snapshot "$RFIXTURE" "$RKEY" "$stage/setup-state-ruxel"
  snapshot "$AFIXTURE" "$AKEY" "$stage/setup-state-ansible"
  diff -u "$stage/setup-state-ruxel" "$stage/setup-state-ansible"
}

if [ "$scenario" = fresh ]; then
  for repetition in $(seq 1 "$REPS"); do
    create_and_prepare_twins "$repetition"
    run_pair "$repetition" "$RFIXTURE" "$RKEY" "$AFIXTURE" "$AKEY"
    destroy_twins
  done
else
  create_and_prepare_twins shared
  untimed_converge
  if [ "$scenario" = check-diff ]; then
    remote_script "$RFIXTURE" "$RKEY" "printf 'drifted\\n' > /tmp/ruxel-fixture-files/block.txt"
    remote_script "$AFIXTURE" "$AKEY" "printf 'drifted\\n' > /tmp/ruxel-fixture-files/block.txt"
    snapshot "$RFIXTURE" "$RKEY" "$stage/pre-ruxel"; snapshot "$AFIXTURE" "$AKEY" "$stage/pre-ansible"
    diff -u "$stage/pre-ruxel" "$stage/pre-ansible"
  elif [ "$scenario" = simulated-rtt ]; then
    netem='device=$(ip route show default | awk '\''{print $5; exit}'\''); tc qdisc replace dev "$device" root netem delay 25ms'
    remote_script "$RFIXTURE" "$RKEY" "$netem"; remote_script "$AFIXTURE" "$AKEY" "$netem"
  fi
  for repetition in $(seq 1 "$REPS"); do
    if [ "$scenario" = one-task-drift ]; then
      remote_script "$RFIXTURE" "$RKEY" 'rm -f /tmp/ruxel-fixture-files/created-by-command'
      remote_script "$AFIXTURE" "$AKEY" 'rm -f /tmp/ruxel-fixture-files/created-by-command'
    fi
    run_pair "$repetition" "$RFIXTURE" "$RKEY" "$AFIXTURE" "$AKEY"
  done
  destroy_twins
fi

fixture_sha="$(sha256sum "$PLAYBOOK" | cut -d' ' -f1)"
crate_version="$(cargo metadata --no-deps --format-version 1 | jq -er '.packages[] | select(.name=="ruxel-cli") | .version')"
ansible_version="$(tools/oracle/.venv/bin/ansible-playbook --version | head -1)"
jq -n -S --arg case "$CASE" --arg playbook "tools/fixture-project/$playbook" \
  --arg fixture_sha "$fixture_sha" --arg controller_sha "$controller_sha" --arg agent_sha "$agent_sha" \
  --arg ansible "$ansible_version" --arg ruxel "$crate_version" --arg agent "$crate_version" \
  --arg rustc "$(mise exec -- rustc --version)" --arg os "$(uname -srm)" --arg kernel "$(uname -r)" \
  --arg kind 'disposable-provider-twin' --arg scenario "$scenario" --arg prepare "$prepare" \
  --arg parity "$PARITY" --arg parity_sha "$(sha256sum "$PARITY" | cut -d' ' -f1)" \
  --argjson repetitions "$REPS" \
  --slurpfile specification "$stage/spec-ruxel.json" \
  '{schema:1,case:$case,playbook:$playbook,fixture_source_sha256:$fixture_sha,
    binaries:{controller_sha256:$controller_sha,agent_sha256:$agent_sha},
    versions:{ansible:$ansible,ruxel:$ruxel,agent:$agent,rustc:$rustc,os:$os,kernel:$kernel},
    fixture:{kind:$kind,specification:$specification[0]},repetitions:$repetitions,
    scenario:$scenario,preparation:$prepare,parity_manifest:$parity,
    parity_manifest_sha256:$parity_sha,
    correctness:{fixture_identity_verified:true,result_parity:true,diff_parity:true,state_parity:true,resources_reaped:true}}' \
  >"$case_dir/manifest.json"
python3 tools/benchmarks/summarize.py "$case_dir"
mkdir -p "$RESULTS_ROOT"
mv "$case_dir" "$final_dir"
echo "PROVIDER BENCHMARK PASS: $CASE ($REPS repetitions per executor)"

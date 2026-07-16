#!/usr/bin/env bash
# Capture acceptance-grade six-host evidence using disposable local containers.
set -euo pipefail
cd "$(dirname "$0")/../.."

CONTROLLER="$(realpath "${1:?ruxel controller binary}")"
AGENT="$(realpath "${2:?x86_64 Linux agent binary}")"
OUTPUT="${3:?output case directory}"
REPETITIONS="${4:-3}"
[[ "$REPETITIONS" =~ ^[0-9]+$ ]] && [ "$REPETITIONS" -ge 3 ] ||
  { echo "repetitions must be an integer >= 3" >&2; exit 2; }
[ ! -e "$OUTPUT" ] || { echo "output already exists: $OUTPUT" >&2; exit 2; }

IMAGE=ruxel-local-fixture:debian12
prefix="ruxel-benchmark-fixture-$$"
work="$(mktemp -d)"
stage="$(mktemp -d)"
case_dir="$stage/six-host"
mkdir -p "$case_dir/logs"
containers=()
cleanup() {
  [ "${#containers[@]}" -eq 0 ] || docker rm -f "${containers[@]}" >/dev/null 2>&1 || true
  rm -rf "$work" "$stage"
}
trap cleanup EXIT

docker build -q -t "$IMAGE" tools/fixtures/docker >/dev/null
ssh-keygen -q -t ed25519 -N '' -f "$work/key"
inventory="$work/inventory.ini"
printf '[multihost]\n' >"$inventory"
for index in $(seq 1 6); do
  name="${prefix}-${index}"
  docker run -d --rm --name "$name" --label ruxel=local-fixture     -v "$work/key.pub:/root/.ssh/authorized_keys:ro" "$IMAGE" >/dev/null
  containers+=("$name")
  ip="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$name")"
  printf 'fixture-%s ansible_ssh_host=%s ansible_ssh_user=root\n' "$index" "$ip" >>"$inventory"
done

for name in "${containers[@]}"; do
  ip="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$name")"
  ready=0
  for _ in $(seq 1 30); do
    if ssh -q -i "$work/key" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new       -o "UserKnownHostsFile=$work/known_hosts" root@"$ip" true; then
      ready=1
      break
    fi
    sleep 1
  done
  [ "$ready" -eq 1 ] || { echo "fixture SSH readiness failed" >&2; exit 1; }
  [ "$(docker inspect -f '{{.Config.Labels.ruxel}}' "$name")" = local-fixture ]
done

run_ruxel() {
  "$CONTROLLER" apply -i "$inventory" --ssh-key "$work/key"     --agent-bin "$AGENT" --output json tools/fixture-project/multihost.yml
}
run_ansible() {
  ANSIBLE_GATHERING=explicit ANSIBLE_HOST_KEY_CHECKING=False   ANSIBLE_SSH_ARGS="-o ControlMaster=no -o ControlPath=none"   ANSIBLE_SSH_COMMON_ARGS="-o IdentitiesOnly=yes -o UserKnownHostsFile=$work/known_hosts -o StrictHostKeyChecking=accept-new"     tools/oracle/.venv/bin/ansible-playbook -f 6 -i "$inventory"     --private-key "$work/key" tools/fixture-project/multihost.yml
}
validate_ruxel() {
  python3 - "$1" <<'PY'
import json, sys
records = [json.loads(line) for line in open(sys.argv[1])]
recaps = [record["host"] for record in records if record.get("event") == "recap"]
assert recaps == [f"fixture-{index}" for index in range(1, 7)], recaps
assert all(record.get("failed") == 0 and record.get("unreachable") == 0
           for record in records if record.get("event") == "recap")
PY
}
validate_ansible() {
  python3 - "$1" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
rows = re.findall(r"^(fixture-[1-6])\s*:.*failed=0.*unreachable=0", text, re.M)
assert sorted(rows) == [f"fixture-{index}" for index in range(1, 7)], rows
PY
}
timed_sample() {
  executor="$1"
  repetition="$2"
  execution_order="$3"
  stdout="$case_dir/logs/${executor}-${repetition}.stdout"
  stderr="$case_dir/logs/${executor}-${repetition}.stderr"
  start="$(python3 -c 'import time; print(time.monotonic_ns())')"
  if [ "$executor" = ansible ]; then
    run_ansible >"$stdout" 2>"$stderr"
    validate_ansible "$stdout"
  else
    run_ruxel >"$stdout" 2>"$stderr"
    validate_ruxel "$stdout"
  fi
  elapsed="$(python3 -c "import time; print(time.monotonic_ns() - $start)")"
  python3 tools/benchmarks/artifact.py "$case_dir" "$executor" "$repetition" \
    "$execution_order" "$elapsed" "$stdout" "$stderr"
}

# Warm identical agent caches before alternating paired samples.
run_ruxel >/dev/null
for repetition in $(seq 1 "$REPETITIONS"); do
  if [ "$((repetition % 2))" -eq 1 ]; then
    timed_sample ansible "$repetition" 1
    timed_sample ruxel "$repetition" 2
  else
    timed_sample ruxel "$repetition" 1
    timed_sample ansible "$repetition" 2
  fi
done

# Repeated connections must leave no SSH session children.
sleep 1
for name in "${containers[@]}"; do
  children="$(docker exec "$name" sh -c 'pgrep -c sshd || true')"
  [ "$children" -le 1 ] || { echo "$name leaked sshd children: $children" >&2; exit 1; }
done

# One stopped host must yield five ordered recaps, one structured unreachable,
# and exit 1. This log is part of the hashed gate metadata, not a timing sample.
kernel="$(docker exec "${containers[0]}" uname -r)"
docker stop "${containers[5]}" >/dev/null
set +e
run_ruxel >"$case_dir/logs/unreachable.stdout" 2>"$case_dir/logs/unreachable.stderr"
unreachable_status=$?
set -e
[ "$unreachable_status" -eq 1 ]
python3 - "$case_dir/logs/unreachable.stdout" <<'PY'
import json, sys
records = [json.loads(line) for line in open(sys.argv[1])]
recaps = [record["host"] for record in records if record.get("event") == "recap"]
unreachable = [record for record in records if record.get("event") == "unreachable"]
assert recaps == [f"fixture-{index}" for index in range(1, 6)], recaps
assert len(unreachable) == 1 and unreachable[0]["host"] == "fixture-6"
assert unreachable[0]["unreachable"] is True
PY
unreachable_stdout_sha="$(python3 - "$case_dir/logs/unreachable.stdout" <<'PY'
import sys
from pathlib import Path
sys.path.insert(0, "tools/benchmarks")
from artifact import sanitize_file
print(sanitize_file(Path(sys.argv[1])))
PY
)"
unreachable_stderr_sha="$(python3 - "$case_dir/logs/unreachable.stderr" <<'PY'
import sys
from pathlib import Path
sys.path.insert(0, "tools/benchmarks")
from artifact import sanitize_file
print(sanitize_file(Path(sys.argv[1])))
PY
)"

# Reap before recording the resources_reaped correctness claim.
docker rm -f "${containers[@]}" >/dev/null 2>&1 || true
[ -z "$(docker ps -aq --filter "name=^/${prefix}-")" ] ||
  { echo "six-host benchmark containers were not reaped" >&2; exit 1; }
containers=()

fixture_sha="$(shasum -a 256 tools/fixture-project/multihost.yml | awk '{print $1}')"
controller_sha="$(shasum -a 256 "$CONTROLLER" | awk '{print $1}')"
agent_sha="$(shasum -a 256 "$AGENT" | awk '{print $1}')"
ansible_version="$(tools/oracle/.venv/bin/ansible-playbook --version | head -1 | sed -E 's/.*core ([^]]+).*/\1/')"
ruxel_version="$("$CONTROLLER" --version | awk '{print $2}')"
rustc_version="$(rustc --version)"
python3 - "$case_dir/manifest.json" "$fixture_sha" "$controller_sha" "$agent_sha"   "$ansible_version" "$ruxel_version" "$rustc_version" "$kernel" "$REPETITIONS"   "$unreachable_stdout_sha" "$unreachable_stderr_sha" <<'PY'
import json, sys
(path, fixture_sha, controller_sha, agent_sha, ansible, ruxel, rustc, kernel,
 repetitions, unreachable_stdout, unreachable_stderr) = sys.argv[1:]
manifest = {
    "schema": 1,
    "case": "six-host",
    "playbook": "tools/fixture-project/multihost.yml",
    "fixture_source_sha256": fixture_sha,
    "binaries": {
        "controller_sha256": controller_sha,
        "agent_sha256": agent_sha,
    },
    "versions": {
        "ansible": ansible,
        "ruxel": ruxel,
        "agent": ruxel,
        "rustc": rustc,
        "os": "Debian 12 local SSH containers",
        "kernel": kernel,
    },
    "fixture": {
        "kind": "six-disposable-local-containers",
        "specification": {
            "image": "ruxel-local-fixture:debian12",
            "host_count": 6,
            "label": "ruxel=local-fixture",
            "ansible_forks": 6,
        },
    },
    "repetitions": int(repetitions),
    "correctness": {
        "fixture_identity_verified": True,
        "result_parity": True,
        "diff_parity": True,
        "state_parity": True,
        "resources_reaped": True,
    },
    "gate_artifacts": {
        "ordered_recaps": True,
        "sshd_leaks": 0,
        "unreachable_exit_status": 1,
        "unreachable_stdout_sha256": unreachable_stdout,
        "unreachable_stderr_sha256": unreachable_stderr,
    },
}
open(path, "w").write(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
PY
python3 tools/benchmarks/summarize.py "$case_dir"
python3 tools/benchmarks/verify.py --case "$case_dir" six-host

mkdir -p "$(dirname "$OUTPUT")"
mv "$case_dir" "$OUTPUT"
echo "SIX HOST BENCHMARK PASS: $OUTPUT repetitions=$REPETITIONS ordered_recaps=6 sshd_leaks=0 unreachable=1"

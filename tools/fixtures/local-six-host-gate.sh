#!/usr/bin/env bash
# Reproducible six-host concurrency/resource gate using disposable SSH containers.
set -euo pipefail
cd "$(dirname "$0")/../.."

AGENT="$(realpath "${1:?x86_64 Linux agent binary}")"
RUNS="${2:-10}"
IMAGE=ruxel-local-fixture:debian12
prefix="ruxel-local-fixture-$$"
work="$(mktemp -d)"
containers=()
cleanup() {
  [ "${#containers[@]}" -eq 0 ] || docker rm -f "${containers[@]}" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

docker build -q -t "$IMAGE" tools/fixtures/docker >/dev/null
ssh-keygen -q -t ed25519 -N '' -f "$work/key"
inventory="$work/inventory.ini"
printf '[multihost]\n' >"$inventory"
for index in $(seq 1 6); do
  name="${prefix}-${index}"
  docker run -d --rm --name "$name" --label ruxel=local-fixture \
    -v "$work/key.pub:/root/.ssh/authorized_keys:ro" "$IMAGE" >/dev/null
  containers+=("$name")
  ip="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$name")"
  printf 'fixture-%s ansible_ssh_host=%s ansible_ssh_user=root\n' "$index" "$ip" >>"$inventory"
done

for name in "${containers[@]}"; do
  ip="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$name")"
  for _ in $(seq 1 30); do
    ssh -q -i "$work/key" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new \
      -o "UserKnownHostsFile=$work/known_hosts" root@"$ip" true && break
    sleep 1
  done
done

run_ruxel() {
  target/debug/ruxel apply -i "$inventory" --ssh-key "$work/key" \
    --agent-bin "$AGENT" --output json "$@" tools/fixture-project/multihost.yml
}

run_ruxel >/dev/null # populate identical content-addressed agent cache on all hosts
single_times="$work/single-times"
for index in $(seq 1 6); do
  single_start="$(python3 -c 'import time; print(time.monotonic())')"
  run_ruxel --limit "fixture-$index" >/dev/null
  python3 -c "import time; print(time.monotonic() - $single_start)" >>"$single_times"
done
slowest="$(python3 -c "print(max(map(float, open('$single_times'))))")"
serial="$(python3 -c "print(sum(map(float, open('$single_times'))))")"
start="$(python3 -c 'import time; print(time.monotonic())')"
run_ruxel >"$work/first.jsonl"
elapsed="$(python3 -c "import time; print(time.monotonic() - $start)")"
python3 - "$work/first.jsonl" "$elapsed" "$slowest" "$serial" <<'PY'
import json, sys
records = [json.loads(line) for line in open(sys.argv[1])]
hosts = [record["host"] for record in records if record.get("event") == "recap"]
assert hosts == [f"fixture-{index}" for index in range(1, 7)], hosts
assert float(sys.argv[2]) <= float(sys.argv[3]) * 2.0, \
    f"six-host={sys.argv[2]}s slowest-host={sys.argv[3]}s"
assert float(sys.argv[2]) < float(sys.argv[4]) / 2, \
    f"six-host={sys.argv[2]}s serial={sys.argv[4]}s"
PY

ansible_start="$(python3 -c 'import time; print(time.monotonic())')"
ANSIBLE_GATHERING=explicit ANSIBLE_HOST_KEY_CHECKING=False \
ANSIBLE_SSH_ARGS="-o ControlMaster=no -o ControlPath=none" \
ANSIBLE_SSH_COMMON_ARGS="-o IdentitiesOnly=yes -o UserKnownHostsFile=$work/known_hosts -o StrictHostKeyChecking=accept-new" \
  tools/oracle/.venv/bin/ansible-playbook -f 6 -i "$inventory" \
  --private-key "$work/key" tools/fixture-project/multihost.yml \
  >"$work/ansible.log"
ansible_elapsed="$(python3 -c "import time; print(time.monotonic() - $ansible_start)")"
grep -q 'failed=0' "$work/ansible.log"

for _ in $(seq 1 "$RUNS"); do
  run_ruxel >/dev/null
done
sleep 1
for name in "${containers[@]}"; do
  children="$(docker exec "$name" sh -c 'pgrep -c sshd || true')"
  [ "$children" -le 1 ] || { echo "$name leaked sshd children: $children" >&2; exit 1; }
done

docker stop "${containers[5]}" >/dev/null
set +e
run_ruxel >"$work/unreachable.jsonl" 2>"$work/unreachable.stderr"
unreachable_status=$?
set -e
[ "$unreachable_status" -eq 1 ]
python3 - "$work/unreachable.jsonl" <<'PY'
import json, sys
records = [json.loads(line) for line in open(sys.argv[1])]
recaps = [record["host"] for record in records if record.get("event") == "recap"]
unreachable = [record for record in records if record.get("event") == "unreachable"]
assert recaps == [f"fixture-{index}" for index in range(1, 6)], recaps
assert len(unreachable) == 1 and unreachable[0]["host"] == "fixture-6", unreachable
assert unreachable[0]["unreachable"] is True
PY

printf 'SIX HOST PASS: ruxel=%ss slowest=%ss serial=%ss ansible=%ss runs=%s ordered_recaps=6 sshd_leaks=0 unreachable=1\n' \
  "$elapsed" "$slowest" "$serial" "$ansible_elapsed" "$((RUNS + 8))"

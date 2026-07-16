#!/usr/bin/env bash
# Create isolated twin targets and run the complete declared fixture matrix.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

RUXEL="$(realpath "${1:?controller binary}")"
AGENT="$(realpath "${2:?x86_64 Linux agent binary}")"
MATRIX=tools/fixtures/parity-matrix.json
only="${RUXEL_MATRIX_ONLY:-}"

python3 - "$MATRIX" <<'PY'
import json, pathlib, sys
matrix = json.load(open(sys.argv[1]))
declared = {entry["playbook"] for entry in matrix}
actual = {path.name for path in pathlib.Path("tools/fixture-project").glob("*.yml")}
assert declared == actual, f"parity matrix drift: missing={sorted(actual-declared)} extra={sorted(declared-actual)}"
PY

active=()
cleanup() {
  for fixture in "${active[@]}"; do
    suffix="${fixture#ruxel-fixture-}"
    if hcloud server list -l ruxel=fixture -o noheader -o columns=name | grep -Fxq "$fixture"; then
      hcloud server delete "$fixture" >/dev/null 2>&1 || true
    fi
    hcloud ssh-key delete "$(session_key_name "$suffix")" >/dev/null 2>&1 || true
    rm -f "${TMPDIR:-/tmp}/${fixture}-ssh" "${TMPDIR:-/tmp}/${fixture}-ssh.pub" \
      "${TMPDIR:-/tmp}/${fixture}-ssh.known_hosts"
  done
}
trap cleanup EXIT

create_fixture() {
  local suffix="$1" output
  active+=("ruxel-fixture-$suffix")
  output="$(tools/fixtures/create.sh "$suffix")"
  CREATED_NAME="$(printf '%s\n' "$output" | sed -n 's/^RUXEL_FIXTURE_NAME=//p' | tail -1)"
  CREATED_KEY="$(printf '%s\n' "$output" | sed -n 's/^RUXEL_FIXTURE_KEY=//p' | tail -1)"
  [ -n "$CREATED_NAME" ] && [ -n "$CREATED_KEY" ] || die "fixture creation output incomplete"
}

while IFS= read -r entry; do
  playbook="$(jq -r .playbook <<<"$entry")"
  stem="${playbook%.yml}"
  [ -z "$only" ] || [ "$only" = "$stem" ] || continue
  group="$(jq -r .group <<<"$entry")"
  prepare="$(jq -r '.prepare // ""' <<<"$entry")"
  dry="$(jq -r 'if .dry then "dry" else "" end' <<<"$entry")"

  create_fixture "matrix-${stem}-ruxel"
  ruxel_fixture="$CREATED_NAME"; ruxel_key="$CREATED_KEY"
  create_fixture "matrix-${stem}-ansible"
  ansible_fixture="$CREATED_NAME"; ansible_key="$CREATED_KEY"

  case "$prepare" in
    "") ;;
    postgresql)
      tools/fixtures/prepare-postgresql.sh "$ruxel_fixture" "$ruxel_key"
      tools/fixtures/prepare-postgresql.sh "$ansible_fixture" "$ansible_key"
      ;;
    storage-*)
      tools/fixtures/prepare-storage.sh "$ruxel_fixture" "$ruxel_key" "$prepare"
      tools/fixtures/prepare-storage.sh "$ansible_fixture" "$ansible_key" "$prepare"
      ;;
    *) die "unknown preparation: $prepare" ;;
  esac

  RUXEL_PROMOTE_CAPTURES=1 tools/fixtures/fresh-parity-gate.sh \
    "$ruxel_fixture" "$ruxel_key" "$ansible_fixture" "$ansible_key" \
    "$RUXEL" "$AGENT" "tools/fixture-project/$playbook" "$dry" "$group"

  tools/fixtures/destroy.sh "$ruxel_fixture"
  tools/fixtures/destroy.sh "$ansible_fixture"
  active=()
done < <(jq -c '.[]' "$MATRIX")

echo "PARITY MATRIX PASS${only:+: $only}"

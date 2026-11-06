#!/usr/bin/env bash

set -euo pipefail

export HCLOUD_CONTEXT="ruxel-fixtures"

readonly MAX_FIXTURES=2
readonly SERVER_TYPE="${RUXEL_FIXTURE_TYPE:-cpx12}"  # smallest x86_64 available (no cx-line in this account)
readonly IMAGE="${RUXEL_FIXTURE_IMAGE:-debian-12}"
readonly LOCATION="${RUXEL_FIXTURE_LOCATION:-sin}"   # cpx12 capacity: sin only (EU shared-x86 unavailable, checked 2026-06-11)
readonly LABEL_SELECTOR="ruxel=fixture"

die() { echo "fixtures: $*" >&2; exit 1; }

require_context() {
  hcloud context list -o noheader 2>/dev/null | grep -q "ruxel-fixtures" \
    || die "hcloud context 'ruxel-fixtures' missing — see docs/OPERATOR-SETUP.md §1; refusing to run"
  hcloud server-type list -o noheader >/dev/null 2>&1 \
    || die "hcloud auth failed for context 'ruxel-fixtures'"
}

fixture_count() {
  hcloud server list -l "$LABEL_SELECTOR" -o noheader 2>/dev/null | wc -l | tr -d ' '
}

session_key_name() { echo "ruxel-fixture-key-$1"; }

fixture_volumes() {
  hcloud volume list -l "$LABEL_SELECTOR" -o noheader -o columns=name 2>/dev/null
}

resolve_fixture() {
  local name="${1:?fixture name required}"
  case "$name" in
    ruxel-fixture-*) ;;
    *) die "refusing target ${name@Q}: expected ruxel-fixture-* provider identity" ;;
  esac
  require_context
  local attempt found=0
  for attempt in 1 2 3 4; do
    if hcloud server list -l "$LABEL_SELECTOR" -o noheader -o columns=name 2>/dev/null \
      | grep -Fxq "$name"; then
      found=1
      break
    fi
    [ "$attempt" -eq 4 ] || sleep "$attempt"
  done
  [ "$found" -eq 1 ] \
    || die "refusing target ${name@Q}: not a labeled fixture in ruxel-fixtures"
  FIXTURE_NAME="$name"
  FIXTURE_IP="$(hcloud server ip "$name")"
  [ -n "$FIXTURE_IP" ] || die "fixture ${name@Q} has no provider address"
  export FIXTURE_NAME FIXTURE_IP
}

require_fixture_key() {
  local key="${1:?fixture key required}"
  [ -f "$key" ] || die "fixture key ${key@Q} does not exist"
  FIXTURE_KEY="$key"
  export FIXTURE_KEY
}

fixture_spec() {
  local name="${1:?fixture name required}"
  resolve_fixture "$name"
  hcloud server describe "$name" -o json | jq -S '{
    image: .image.name,
    location: .location.name,
    primary_disk_size,
    server_type: .server_type.name,
    volume_count: (.volumes | length)
  }'
}

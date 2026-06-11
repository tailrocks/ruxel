#!/usr/bin/env bash
# Shared helpers for ruxel fixture VMs (GOAL.md safety rules 2 & 3).
#
# Every fixture resource lives in the Hetzner Cloud project behind the local
# `ruxel-fixtures` hcloud context and carries the label ruxel=fixture. These
# scripts refuse to run against anything else: targets are only ever taken
# from `hcloud server list` output of that context.

set -euo pipefail

export HCLOUD_CONTEXT="ruxel-fixtures"

# Hard caps (GOAL.md rule 3).
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

#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
RUXEL="$(realpath "${1:?ruxel binary}")"
AGENT="$(realpath "${2:?x86_64 Linux agent binary}")"
suffix="chaos-$(date +%s)-$$"
fixture=""
cleanup() {
  [ -z "$fixture" ] || tools/fixtures/destroy.sh "$fixture" || true
}
trap cleanup EXIT INT TERM

source <(tools/fixtures/create.sh "$suffix")
fixture="$RUXEL_FIXTURE_NAME"
tools/chaos/gate.sh "$fixture" "$RUXEL_FIXTURE_KEY" "$RUXEL" "$AGENT"

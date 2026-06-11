#!/usr/bin/env bash
# Run the M2 transport gate against a fixture VM: two single-connect
# processes — cold (may upload), then warm (must skip upload, < 1 s).
#
# Usage: tools/fixtures/gate.sh <dest> <keyfile> <agent-binary>
set -euo pipefail
cd "$(dirname "$0")/../.."

DEST="${1:?ssh destination}"
KEY="${2:?keyfile}"
AGENT="${3:?agent binary}"

echo "== gate run 1 (cold)"
RUXEL_TEST_SSH_DEST="$DEST" RUXEL_TEST_SSH_KEY="$KEY" RUXEL_TEST_AGENT_BIN="$AGENT" \
  cargo test -p ruxel-cli --test transport_gate -- --ignored --nocapture

echo "== gate run 2 (warm: no re-upload, < 1 s)"
RUXEL_TEST_SSH_DEST="$DEST" RUXEL_TEST_SSH_KEY="$KEY" RUXEL_TEST_AGENT_BIN="$AGENT" \
RUXEL_TEST_EXPECT_NO_UPLOAD=1 RUXEL_TEST_EXPECT_FAST=1 \
  cargo test -p ruxel-cli --test transport_gate -- --ignored --nocapture

echo "gate: PASS"

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BIN="$(mktemp -d)"
KEY="$(mktemp)"
trap 'rm -rf "$BIN" "$KEY"' EXIT
ln -s "$ROOT/tools/fixtures/tests/fake-hcloud" "$BIN/hcloud"
export PATH="$BIN:$PATH"

source "$ROOT/tools/fixtures/lib.sh"

resolve_fixture ruxel-fixture-valid
[ "$FIXTURE_IP" = 192.0.2.10 ]
require_fixture_key "$KEY"
[ "$FIXTURE_KEY" = "$KEY" ]

if (resolve_fixture 192.0.2.10) 2>/dev/null; then
  echo "raw address was accepted" >&2
  exit 1
fi
if (resolve_fixture ruxel-fixture-unlabeled) 2>/dev/null; then
  echo "unlabeled fixture was accepted" >&2
  exit 1
fi
if (require_fixture_key "$KEY.missing") 2>/dev/null; then
  echo "missing key was accepted" >&2
  exit 1
fi

echo "fixture safety: PASS"

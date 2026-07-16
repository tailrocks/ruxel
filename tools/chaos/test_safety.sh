#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$(mktemp -d)"
KEY="$(mktemp)"
RUXEL="$(mktemp)"
AGENT="$(mktemp)"
trap 'rm -rf "$BIN" "$KEY" "$RUXEL" "$AGENT"' EXIT
ln -s "$ROOT/tools/fixtures/tests/fake-hcloud" "$BIN/hcloud"
cat >"$BIN/ssh" <<'SH'
#!/usr/bin/env bash
echo "safety test contacted ssh" >&2
exit 99
SH
chmod +x "$BIN/ssh" "$RUXEL" "$AGENT"
export PATH="$BIN:$PATH"

RUXEL_CHAOS_VALIDATE_ONLY=1 "$ROOT/tools/chaos/gate.sh" \
  ruxel-fixture-valid "$KEY" "$RUXEL" "$AGENT"
if RUXEL_CHAOS_VALIDATE_ONLY=1 "$ROOT/tools/chaos/gate.sh" \
  192.0.2.10 "$KEY" "$RUXEL" "$AGENT" 2>/dev/null; then
  echo "raw target passed chaos safety boundary" >&2
  exit 1
fi
if RUXEL_CHAOS_VALIDATE_ONLY=1 "$ROOT/tools/chaos/gate.sh" \
  ruxel-fixture-unlabeled "$KEY" "$RUXEL" "$AGENT" 2>/dev/null; then
  echo "unlabeled target passed chaos safety boundary" >&2
  exit 1
fi
echo "chaos safety: PASS"

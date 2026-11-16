#!/usr/bin/env bash
# Poll PR #100 merge gate: unresolved review-thread count (paginated) + per-check conclusions.
# Runs every 60s for up to 12 minutes (12 iterations). Read-only.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
LOG="${1:-/tmp/pr100_gate.log}"
: > "$LOG"
for i in $(seq 1 12); do
  echo "=== iteration $i $(date -u +%FT%TZ) ===" >> "$LOG"
  python3 .pi/poll100.py > /tmp/pr100_poll_iter.json 2>>"$LOG" || { echo "poll failed" >> "$LOG"; }
  python3 - >> "$LOG" <<'PY'
import json
try:
    d=json.load(open('/tmp/pr100_poll_iter.json'))
except Exception as e:
    print("parse error", e); raise SystemExit
print("head=%s total_threads=%s fetched=%s pages=%s unresolved=%s" % (
    d['head_sha'], d['total_threads'], d['fetched_threads'], d['pages'], d['unresolved_count']))
for t in d['unresolved']:
    print("  OPEN %s %s:%s outdated=%s | %s" % (t['id'], t['path'], t['line'], t['outdated'], t['body'][:120]))
ns=[c for c in d['checks_not_success']]
print("checks_not_success(%d): %s" % (len(ns), ", ".join("%s=%s"%(c['name'],c['conclusion']) for c in ns)))
PY
  sleep 60
done
echo "=== poll window complete $(date -u +%FT%TZ) ===" >> "$LOG"

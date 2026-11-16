#!/usr/bin/env bash
# Background poll loop for PR #96: 12 iterations, 60s apart.
LOG=/tmp/pr96_loop.log
: > "$LOG"
for i in $(seq 1 12); do
  ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  out=$(cd /Users/donbeave/Projects/ChainArgos/servers-goal/ruxel-w3-impl && python3 .pi/poll96.py 2>&1)
  rc=$?
  if [[ $rc -ne 0 ]]; then
    echo "$ts ITER=$i ERROR rc=$rc ${out:0:300}" >> "$LOG"
  else
    echo "$ts ITER=$i" >> "$LOG"
    echo "$out" | python3 -c '
import json,sys
d=json.load(sys.stdin)
print("  head=%s total=%s fetched=%s pages=%s unresolved=%s checks=%d not_ok=%s" % (
  d["head_sha"][:12], d["total_threads"], d["fetched_threads"], d["pages"],
  d["unresolved_count"], len(d["checks"]),
  [(c["name"],c["conclusion"]) for c in d["checks_not_success"]]))
for t in d["unresolved"]:
    print("   OPEN %s %s:%s %s" % (t["id"], t["path"], t["line"], t["body"][:90]))
' >> "$LOG"
  fi
  sleep 60
done
echo "LOOP_DONE $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$LOG"

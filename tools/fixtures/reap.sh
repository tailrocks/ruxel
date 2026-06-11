#!/usr/bin/env bash
# List and destroy ALL leftover fixture resources (label ruxel=fixture).
# Safe to run at the start and end of every session (GOAL.md rule 3).
#
# Usage: tools/fixtures/reap.sh [--dry-run]

source "$(dirname "$0")/lib.sh"

require_context

dry="${1:-}"

servers="$(hcloud server list -l "$LABEL_SELECTOR" -o noheader -o columns=name || true)"
keys="$(hcloud ssh-key list -l "$LABEL_SELECTOR" -o noheader -o columns=name || true)"

if [ -z "$servers" ] && [ -z "$keys" ]; then
  echo "no fixture leftovers"
  exit 0
fi

for s in $servers; do
  echo "leftover server: $s"
  [ "$dry" = "--dry-run" ] || hcloud server delete "$s" >/dev/null
done
for k in $keys; do
  echo "leftover ssh key: $k"
  [ "$dry" = "--dry-run" ] || hcloud ssh-key delete "$k" >/dev/null
done
[ "$dry" = "--dry-run" ] && echo "(dry run — nothing deleted)" || echo "reaped"

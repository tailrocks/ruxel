#!/usr/bin/env bash
# Destroy one fixture VM and its ephemeral key. Refuses non-fixture targets.
#
# Usage: tools/fixtures/destroy.sh <ruxel-fixture-name>

source "$(dirname "$0")/lib.sh"

require_context

name="${1:?usage: destroy.sh <ruxel-fixture-name>}"
case "$name" in
  ruxel-fixture-*) ;;
  *) die "refusing to destroy ${name@Q}: not a ruxel-fixture-* name" ;;
esac

hcloud server list -l "$LABEL_SELECTOR" -o noheader -o columns=name | grep -qx "$name" \
  || die "refusing: ${name@Q} is not a labeled fixture in this project"

hcloud server delete "$name" >/dev/null
suffix="${name#ruxel-fixture-}"
hcloud ssh-key delete "$(session_key_name "$suffix")" >/dev/null 2>&1 || true
rm -f "${TMPDIR:-/tmp}/${name}-ssh" "${TMPDIR:-/tmp}/${name}-ssh.pub" \
      "${TMPDIR:-/tmp}/${name}-ssh.known_hosts"
echo "destroyed ${name}"

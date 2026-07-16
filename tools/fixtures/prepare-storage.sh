#!/usr/bin/env bash
# Provision isolated loop-backed disks for synthetic storage fixtures.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

FIXTURE="${1:?provider fixture name}"
KEY="${2:?fixture key}"
PROFILE="${3:?storage-ext4|storage-xfs|storage-two-tier}"
resolve_fixture "$FIXTURE"
require_fixture_key "$KEY"

case "$PROFILE" in
  storage-ext4) names=(disk-1 disk-2) ;;
  storage-xfs) names=(disk-1 disk-2 disk-3) ;;
  storage-two-tier) names=(data-1 data-2 hot-1) ;;
  *) die "unknown storage profile: $PROFILE" ;;
esac

ssh -i "$KEY" -o IdentitiesOnly=yes \
  -o "UserKnownHostsFile=${KEY}.known_hosts" -o StrictHostKeyChecking=accept-new \
  "root@${FIXTURE_IP}" bash -se -- "${names[@]}" <<'REMOTE'
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq lvm2 xfsprogs
mkdir -p /var/lib/ruxel-fixture-disks /dev/disk/by-id
for name in "$@"; do
  image="/var/lib/ruxel-fixture-disks/${name}.img"
  truncate -s 384M "$image"
  device="$(losetup --find --show "$image")"
  ln -sfn "$device" "/dev/disk/by-id/ruxel-fixture-${name}"
done
REMOTE

echo "Storage fixture $PROFILE ready on provider identity: $FIXTURE"

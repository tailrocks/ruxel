#!/usr/bin/env bash
# Capture deterministic observable state from a labeled disposable fixture.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

FIXTURE="${1:?provider fixture name}"
KEY="${2:?fixture key}"
OUTPUT="${3:?output path}"
resolve_fixture "$FIXTURE"
require_fixture_key "$KEY"

ssh -o IdentitiesOnly=yes \
  -o UserKnownHostsFile="${KEY}.known_hosts" \
  -o StrictHostKeyChecking=accept-new \
  -i "$KEY" "root@${FIXTURE_IP}" 'bash -s' > "$OUTPUT" <<'REMOTE'
set -euo pipefail

echo '[managed-paths]'
for root in /tmp/ruxel-fixture-* /mnt/ruxel-fixture* /var/lib/ruxelfixture; do
  [ -e "$root" ] || [ -L "$root" ] || continue
  find "$root" -xdev -printf '%y\t%p\t%m\t%U\t%G\t%l\n' 2>/dev/null
done | LC_ALL=C sort

echo '[managed-content]'
for root in /tmp/ruxel-fixture-* /mnt/ruxel-fixture* /var/lib/ruxelfixture; do
  [ -e "$root" ] || continue
  find "$root" -xdev -type f \
    ! -name 'ruxel-fixture-http.log' \
    ! -name 'ruxel-fixture-http.pid' \
    ! -path '*/.git/logs/*' \
    -print0 2>/dev/null
done | LC_ALL=C sort -z | xargs -0 -r sha256sum

echo '[packages]'
dpkg-query -W -f='${Package}\t${Version}\t${Status}\n' \
  ca-certificates git openssh-server 2>/dev/null | LC_ALL=C sort || true

echo '[accounts]'
getent passwd ruxelfixture 2>/dev/null || true
getent group ruxelfixture 2>/dev/null || true
getent passwd ruxelfixture-absent 2>/dev/null || true
getent group ruxelfixture-absent 2>/dev/null || true

echo '[sysctl]'
for key in net.ipv4.ip_forward vm.swappiness; do
  printf '%s=' "$key"
  sysctl -n "$key" 2>/dev/null || true
done

echo '[firewall]'
iptables-save 2>/dev/null | grep -F 'ruxel synthetic fixture' | LC_ALL=C sort || true

echo '[mounts]'
findmnt -rn -o TARGET,SOURCE,FSTYPE,OPTIONS 2>/dev/null \
  | grep -E '^/mnt/ruxel-fixture' | LC_ALL=C sort || true

echo '[lvm]'
pvs --noheadings --separator '|' -o pv_name,vg_name 2>/dev/null | sed 's/^ *//;s/ *$//' | LC_ALL=C sort || true
vgs --noheadings --separator '|' -o vg_name,vg_size 2>/dev/null | sed 's/^ *//;s/ *$//' | LC_ALL=C sort || true
lvs --noheadings --separator '|' -o vg_name,lv_name,lv_size 2>/dev/null | sed 's/^ *//;s/ *$//' | LC_ALL=C sort || true

echo '[postgresql]'
if command -v psql >/dev/null && id postgres >/dev/null 2>&1; then
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT datname||'|'||pg_get_userbyid(datdba) FROM pg_database WHERE datname LIKE 'ruxel_fixture%' ORDER BY 1" 2>/dev/null || true
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT nspname||'|'||pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname LIKE 'ruxel_fixture%' ORDER BY 1" 2>/dev/null || true
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT rolname||'|'||rolcanlogin FROM pg_roles WHERE rolname LIKE 'ruxel_fixture%' ORDER BY 1" 2>/dev/null || true
fi
REMOTE

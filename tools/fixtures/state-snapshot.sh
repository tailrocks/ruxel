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
  find "$root" -xdev -path '*/.ansible' -prune -o \
    -printf '%y\t%p\t%m\t%U\t%G\t%l\n' 2>/dev/null
done | LC_ALL=C sort

echo '[managed-content]'
for root in /tmp/ruxel-fixture-* /mnt/ruxel-fixture* /var/lib/ruxelfixture; do
  [ -e "$root" ] || continue
  find "$root" -xdev -path '*/.ansible' -prune -o -type f \
    ! -name 'ruxel-fixture-http.log' \
    ! -name 'ruxel-fixture-http.pid' \
    ! -path '*/.git/index' \
    ! -path '*/.git/logs/*' \
    -print0 2>/dev/null
done | LC_ALL=C sort -z | xargs -0 -r sha256sum

echo '[packages]'
dpkg-query -W -f='${Package}\t${Version}\t${Status}\n' \
  ca-certificates git openssh-server 2>/dev/null | LC_ALL=C sort || true

echo '[repositories]'
for file in /etc/apt/sources.list /etc/apt/sources.list.d/*; do
  [ -f "$file" ] || continue
  printf '%s\t' "$file"
  sha256sum "$file" | cut -d' ' -f1
done | LC_ALL=C sort

echo '[services]'
for unit in ssh.service ruxel-fixture.service; do
  printf '%s\t' "$unit"
  systemctl is-enabled "$unit" 2>/dev/null || true
  printf '%s\t' "$unit"
  systemctl is-active "$unit" 2>/dev/null || true
done

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

echo '[filesystems]'
blkid -o export 2>/dev/null | awk '
  /^DEVNAME=\/dev\/(mapper\/)?ruxel_fixture/ { keep=1 }
  /^DEVNAME=/ && $0 !~ /^DEVNAME=\/dev\/(mapper\/)?ruxel_fixture/ { keep=0 }
  keep && /^(DEVNAME|TYPE)=/ { print }
  keep && /^UUID=/ { print "UUID=<present>" }
' | LC_ALL=C sort

echo '[git]'
for directory in $(find /tmp/ruxel-fixture-* /var/lib/ruxelfixture \
    -type d -name .git -prune 2>/dev/null | LC_ALL=C sort); do
  root="${directory%/.git}"
  echo "root=$root"
  git -C "$root" rev-parse HEAD 2>/dev/null || true
  git -C "$root" status --porcelain=v1 --untracked-files=all 2>/dev/null || true
  git -C "$root" show-ref 2>/dev/null | LC_ALL=C sort || true
  git -C "$root" ls-files --stage 2>/dev/null | LC_ALL=C sort || true
  git -C "$root" remote -v 2>/dev/null | LC_ALL=C sort -u || true
done

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
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT extname||'|'||extversion FROM pg_extension ORDER BY 1" 2>/dev/null || true
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT schemaname||'|'||tablename||'|'||tableowner FROM pg_tables WHERE schemaname LIKE 'ruxel_fixture%' ORDER BY 1" 2>/dev/null || true
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT rolname||'|'||rolsuper||'|'||rolcreaterole||'|'||rolcreatedb||'|'||rolinherit||'|'||rolcanlogin||'|'||rolreplication||'|'||rolbypassrls FROM pg_roles WHERE rolname LIKE 'ruxel_fixture%' ORDER BY 1" 2>/dev/null || true
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT roleid::regrole||'|'||member::regrole||'|'||admin_option FROM pg_auth_members WHERE roleid::regrole::text LIKE 'ruxel_fixture%' OR member::regrole::text LIKE 'ruxel_fixture%' ORDER BY 1" 2>/dev/null || true
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT 'database|'||datname||'|'||coalesce(datacl::text,'') FROM pg_database WHERE datname LIKE 'ruxel_fixture%' UNION ALL SELECT 'schema|'||nspname||'|'||coalesce(nspacl::text,'') FROM pg_namespace WHERE nspname LIKE 'ruxel_fixture%' ORDER BY 1" 2>/dev/null || true
  runuser -u postgres -- psql -X -qAt -p 40000 -d postgres -c \
    "SELECT defaclrole::regrole||'|'||coalesce(n.nspname,'')||'|'||defaclobjtype||'|'||defaclacl::text FROM pg_default_acl d LEFT JOIN pg_namespace n ON n.oid=d.defaclnamespace WHERE defaclrole::regrole::text LIKE 'ruxel_fixture%' ORDER BY 1" 2>/dev/null || true
fi
REMOTE

#!/usr/bin/env bash
# Provision the synthetic PostgreSQL fixture on a provider-verified Debian target.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/fixtures/lib.sh

FIXTURE="${1:?provider fixture name}"
KEY="${2:?fixture key}"
resolve_fixture "$FIXTURE"
require_fixture_key "$KEY"

ssh -i "$KEY" -o IdentitiesOnly=yes \
  -o "UserKnownHostsFile=${KEY}.known_hosts" -o StrictHostKeyChecking=accept-new \
  "root@${FIXTURE_IP}" 'bash -se' <<'REMOTE'
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq postgresql python3-psycopg2
version="$(ls /etc/postgresql | sort -V | tail -1)"
cluster="$(pg_lsclusters -h | awk '$1 == v { print $2; exit }' v="$version")"
if [ -z "$cluster" ]; then
  cluster=ruxel_fixture
  pg_createcluster "$version" "$cluster" --port 40000 --start
else
  config="/etc/postgresql/${version}/${cluster}/postgresql.conf"
  sed -i -E 's/^#?port = .*/port = 40000/' "$config"
  pg_ctlcluster "$version" "$cluster" restart
fi
pg_isready -q -p 40000
REMOTE

echo "PostgreSQL fixture ready on provider identity: $FIXTURE"

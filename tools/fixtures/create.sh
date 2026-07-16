#!/usr/bin/env bash
# Create one fixture VM with an ephemeral SSH key. Prints connection info as
# shell-evaluable lines:   RUXEL_FIXTURE_NAME=… RUXEL_FIXTURE_IP=… RUXEL_FIXTURE_KEY=…
#
# Usage: tools/fixtures/create.sh [name-suffix]

source "$(dirname "$0")/lib.sh"

require_context

count="$(fixture_count)"
[ "$count" -lt "$MAX_FIXTURES" ] \
  || die "fixture cap reached ($count/$MAX_FIXTURES) — run tools/fixtures/reap.sh first"

suffix="${1:-$(date +%s)}"
name="ruxel-fixture-${suffix}"
keyfile="${TMPDIR:-/tmp}/${name}-ssh"

ssh-keygen -t ed25519 -N "" -C "$name" -f "$keyfile" -q
trap 'rm -f "$keyfile" "$keyfile.pub"' ERR

hcloud ssh-key create --name "$(session_key_name "$suffix")" \
  --label "$LABEL_SELECTOR" --public-key-from-file "$keyfile.pub" >/dev/null

hcloud server create \
  --name "$name" \
  --type "$SERVER_TYPE" \
  --image "$IMAGE" \
  --location "$LOCATION" \
  --label "$LABEL_SELECTOR" \
  --ssh-key "$(session_key_name "$suffix")" >/dev/null

ip="$(hcloud server ip "$name")"

# Wait through cloud-init. Fresh images may briefly accept SSH, then restart
# sshd while cloud-init finishes; one successful probe is not readiness.
cloud_init_ready=0
for _ in $(seq 1 60); do
  if ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=3 \
        -o IdentitiesOnly=yes \
        -o UserKnownHostsFile="${keyfile}.known_hosts" \
        -i "$keyfile" "root@${ip}" \
        'cloud-init status --wait >/dev/null 2>&1 || true' 2>/dev/null; then
    cloud_init_ready=1
    break
  fi
  sleep 2
done
[ "$cloud_init_ready" -eq 1 ] || die "fixture SSH/cloud-init readiness timed out"

stable=0
for _ in $(seq 1 30); do
  if ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=3 \
        -o IdentitiesOnly=yes \
        -o UserKnownHostsFile="${keyfile}.known_hosts" \
        -i "$keyfile" "root@${ip}" true 2>/dev/null; then
    stable=$((stable + 1))
    [ "$stable" -ge 3 ] && break
  else
    stable=0
  fi
  sleep 2
done
[ "$stable" -ge 3 ] || die "fixture SSH did not remain stable"

echo "RUXEL_FIXTURE_NAME=${name}"
echo "RUXEL_FIXTURE_IP=${ip}"
echo "RUXEL_FIXTURE_KEY=${keyfile}"
echo "RUXEL_FIXTURE_SSH_OPTS='-i ${keyfile} -o IdentitiesOnly=yes -o UserKnownHostsFile=${keyfile}.known_hosts -o StrictHostKeyChecking=accept-new'"

#!/bin/bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: tests/aws-emulator/run.sh

Simulates a Harbor registry pod that reaches S3 through imds-bridge:
AWS Emulator (S3, STS), a stand-in Pod Identity Agent, the imds-bridge image,
and the Harbor registry image share one network namespace, like containers
of one pod. Images are pushed and pulled through the registry with no static
key in its config.

Environment:
  CONTAINER_TOOL  docker or podman (default: whichever is installed, docker first)
  EMULATOR_PORT   host and container port of the emulator (default: 9297)
  REGISTRY_PORT   host port of the registry (default: 15000)
  METRICS_PORT    host port of the imds-bridge admin listener (default: 19090)
  REGISTRY_IMAGE  registry under test (default: docker.io/goharbor/registry-photon:v2.16.0)
EOF
}

log() { printf '%s\n' "$*" >&2; }
fail() {
  log "FAIL: $*"
  exit 1
}

case "${1:-}" in
  -h | --help)
    usage
    exit 0
    ;;
esac

here="$(cd "$(dirname "$0")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
port="${EMULATOR_PORT:-9297}"
registry_port="${REGISTRY_PORT:-15000}"
metrics_port="${METRICS_PORT:-19090}"
registry_image="${REGISTRY_IMAGE:-docker.io/goharbor/registry-photon:v2.16.0}"
agent_image="public.ecr.aws/docker/library/python:3.13-alpine"
emulator_image="imds-bridge-aws-emulator:local"
bridge_image="imds-bridge:emulator-test"
agent_port=18023
bucket="harbor-registry"
region="us-east-1"

tool="${CONTAINER_TOOL:-}"
if [ -z "$tool" ]; then
  if command -v docker >/dev/null 2>&1; then
    tool=docker
  elif command -v podman >/dev/null 2>&1; then
    tool=podman
  else
    fail "docker or podman is required"
  fi
fi
for cmd in "$tool" curl cargo make; do
  command -v "$cmd" >/dev/null 2>&1 || fail "$cmd is required"
done

case "$(uname -m)" in
  arm64 | aarch64)
    rpm_arch=aarch64
    arch=arm64
    ;;
  x86_64 | amd64)
    rpm_arch=x86_64
    arch=amd64
    ;;
  *) fail "unsupported architecture: $(uname -m)" ;;
esac

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

work="$(mktemp -d)"
home="$work/home"
shared="$work/shared"
mkdir -p "$home" "$shared"
chmod 755 "$work" "$shared"

suffix="$$"
emulator="imds-bridge-emulator-$suffix"
agent="imds-bridge-agent-$suffix"
bridge="imds-bridge-sidecar-$suffix"
registry="imds-bridge-registry-$suffix"

cleanup() {
  status=$?
  if [ "$status" -ne 0 ]; then
    for name in "$bridge" "$registry"; do
      log "--- logs: $name"
      "$tool" logs "$name" 2>&1 | tail -n 30 >&2 || true
    done
  fi
  "$tool" rm -f "$registry" "$bridge" "$agent" "$emulator" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

log "building imds-bridge image (linux/$arch)"
make -C "$crate" "zigbuild-$arch" >/dev/null
"$tool" build --quiet --platform "linux/$arch" -t "$bridge_image" "$crate" >/dev/null

log "building $emulator_image ($rpm_arch)"
"$tool" build --quiet --build-arg "ARCH=$rpm_arch" -t "$emulator_image" "$here" >/dev/null

user_args=()
if [ "$(uname -s)" = Linux ]; then
  user_args=(--user "$(id -u):$(id -g)")
fi

# The emulator certificate only covers loopback, so every other container
# joins its network namespace and reaches it on 127.0.0.1.
log "starting AWS Emulator on 127.0.0.1:$port"
"$tool" run -d --name "$emulator" --memory=2g --memory-swap=2g \
  ${user_args[@]+"${user_args[@]}"} \
  -p "127.0.0.1:$port:$port" \
  -p "127.0.0.1:$registry_port:5000" \
  -p "127.0.0.1:$metrics_port:9090" \
  -v "$home:/home/emulator/.aws/emulator" \
  "$emulator_image" -p "$port" --telemetry off >/dev/null

endpoint="https://127.0.0.1:$port"
ca="$home/$port/ca.pem"
wait_for() {
  description="$1"
  shift
  for _ in $(seq 60); do
    if "$@" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  fail "$description did not become ready"
}
emulator_healthy() { [ -f "$ca" ] && curl -sf --cacert "$ca" "$endpoint/__admin/health"; }
wait_for "emulator" emulator_healthy

access_key="$(awk -F' = ' '/aws_access_key_id/{print $2}' "$home/$port/emulator_credentials")"
secret_key="$(awk -F' = ' '/aws_secret_access_key/{print $2}' "$home/$port/emulator_credentials")"
aws_curl() {
  service="$1"
  shift
  curl -sS --fail-with-body --cacert "$ca" \
    --aws-sigv4 "aws:amz:$region:$service" --user "$access_key:$secret_key" "$@"
}

cp "$ca" "$shared/ca.pem"
printf 'pod-identity-token-%s\n' "$suffix" >"$shared/token"

# Temporary credentials from STS, in the Pod Identity Agent response format.
mint_credentials() {
  xml="$(aws_curl sts -d 'Action=GetSessionToken&Version=2011-06-15&DurationSeconds=900' "$endpoint")"
  field() { printf '%s' "$xml" | sed -n "s:.*<$1>\([^<]*\)</$1>.*:\1:p"; }
  [ -n "$(field SessionToken)" ] || fail "STS returned no session token"
  printf '{"AccessKeyId":"%s","SecretAccessKey":"%s","Token":"%s","AccountId":"000000000000","Expiration":"%s"}' \
    "$(field AccessKeyId)" "$(field SecretAccessKey)" "$(field SessionToken)" "$(field Expiration)" \
    >"$shared/credentials.json.tmp"
  mv "$shared/credentials.json.tmp" "$shared/credentials.json"
  field AccessKeyId
}

log "creating bucket $bucket and minting session credentials"
aws_curl s3 -o /dev/null -X PUT "$endpoint/$bucket"
first_key="$(mint_credentials)"
chmod 644 "$shared"/*

cat >"$shared/config.yml" <<EOF
version: 0.1
log:
  level: info
storage:
  s3:
    region: $region
    bucket: $bucket
    regionendpoint: $endpoint
    secure: true
    v4auth: true
http:
  addr: :5000
EOF

log "starting the stand-in Pod Identity Agent"
"$tool" run -d --name "$agent" --network "container:$emulator" \
  -v "$here/agent.py:/agent.py:ro" -v "$shared:/shared:ro" \
  "$agent_image" python /agent.py /shared "$agent_port" >/dev/null

# The margin leaves the reported expiration 15 seconds away, so the registry
# has to come back for credentials during the test.
log "starting imds-bridge"
token_path=/var/run/secrets/pods.eks.amazonaws.com/serviceaccount/eks-pod-identity-token
"$tool" run -d --name "$bridge" --network "container:$emulator" \
  -e "AWS_CONTAINER_CREDENTIALS_FULL_URI=http://127.0.0.1:$agent_port/v1/credentials" \
  -e "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE=$token_path" \
  -e ROLE_NAME=harbor-registry \
  -e REFRESH_INTERVAL_SECONDS=1 \
  -e EXPIRY_MARGIN_SECONDS=885 \
  -v "$shared/token:$token_path:ro" \
  "$bridge_image" >/dev/null
admin="http://127.0.0.1:$metrics_port"
wait_for "imds-bridge" curl -sf "$admin/readyz"

log "starting $registry_image with no static key"
"$tool" run -d --name "$registry" --network "container:$emulator" \
  -e AWS_EC2_METADATA_SERVICE_ENDPOINT=http://127.0.0.1:1338 \
  -e SSL_CERT_FILE=/shared/ca.pem \
  -v "$shared:/shared:ro" -v "$shared/config.yml:/etc/registry/config.yml:ro" \
  --entrypoint /usr/bin/registry_DO_NOT_USE_GC \
  "$registry_image" serve /etc/registry/config.yml >/dev/null
reg="http://127.0.0.1:$registry_port"
wait_for "registry" curl -sf "$reg/v2/"

upload_blob() {
  repo="$1"
  file="$2"
  digest="sha256:$(sha256 "$file")"
  location="$(curl -sS --fail-with-body -X POST -D - -o /dev/null "$reg/v2/$repo/blobs/uploads/" |
    awk 'tolower($1) == "location:" {print $2}' | tr -d '\r')"
  case "$location" in
    http*) ;;
    *) location="$reg$location" ;;
  esac
  case "$location" in
    *\?*) location="$location&digest=$digest" ;;
    *) location="$location?digest=$digest" ;;
  esac
  curl -sS --fail-with-body -o /dev/null -X PUT \
    -H 'Content-Type: application/octet-stream' --data-binary "@$file" "$location"
  printf '%s' "$digest"
}

# Pushes a one-layer image, pulls it back, and checks the layer landed in S3.
push_and_pull() {
  repo="$1"
  layer="$work/layer"
  config="$work/config.json"
  manifest="$work/manifest.json"
  printf 'layer of %s at %s\n' "$repo" "$(date +%s)" >"$layer"
  printf '{"architecture":"%s","os":"linux","rootfs":{"type":"layers","diff_ids":[]}}' "$arch" >"$config"

  layer_digest="$(upload_blob "$repo" "$layer")"
  config_digest="$(upload_blob "$repo" "$config")"
  printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"%s","size":%s},"layers":[{"mediaType":"application/vnd.oci.image.layer.v1.tar","digest":"%s","size":%s}]}' \
    "$config_digest" "$(wc -c <"$config" | tr -d ' ')" \
    "$layer_digest" "$(wc -c <"$layer" | tr -d ' ')" >"$manifest"
  curl -sS --fail-with-body -o /dev/null -X PUT \
    -H 'Content-Type: application/vnd.oci.image.manifest.v1+json' \
    --data-binary "@$manifest" "$reg/v2/$repo/manifests/v1"

  curl -sS --fail-with-body -o "$work/pulled-manifest" \
    -H 'Accept: application/vnd.oci.image.manifest.v1+json' "$reg/v2/$repo/manifests/v1"
  cmp -s "$manifest" "$work/pulled-manifest" || fail "$repo: pulled manifest differs"

  redirects="$(curl -sS --fail-with-body -L --cacert "$ca" -o "$work/pulled-layer" \
    -w '%{num_redirects}' "$reg/v2/$repo/blobs/$layer_digest")"
  cmp -s "$layer" "$work/pulled-layer" || fail "$repo: pulled layer differs"

  hex="${layer_digest#sha256:}"
  key="docker/registry/v2/blobs/sha256/${hex:0:2}/$hex/data"
  aws_curl s3 "$endpoint/$bucket?list-type=2&prefix=$key" | grep -q "<Key>$key</Key>" ||
    fail "$repo: layer object $key is not in the bucket"
  log "ok: $repo pushed and pulled (layer served through $redirects redirect to S3)"
}

credential_requests() {
  curl -sf "$admin/metrics" |
    awk '/^imds_bridge_requests_total\{path="[^"]*\{role\}",code="200"\}/ {print $2}' | grep . || echo 0
}

push_and_pull e2e/first
before="$(credential_requests)"
[ "$before" -ge 1 ] || fail "the registry never asked imds-bridge for credentials"
log "ok: registry obtained credentials from imds-bridge ($before request)"

log "rotating the credentials behind the agent"
second_key="$(mint_credentials)"
chmod 644 "$shared/credentials.json"
[ "$first_key" != "$second_key" ] || fail "STS returned the same access key twice"
bridge_rotated() { [ "$("$tool" logs "$bridge" 2>&1 | grep -c 'Loaded new credentials')" -ge 2 ]; }
wait_for "credential rotation in imds-bridge" bridge_rotated

registry_refetched() {
  curl -sf -o /dev/null "$reg/v2/e2e/first/manifests/v1"
  [ "$(credential_requests)" -gt "$before" ]
}
wait_for "credential refresh by the registry" registry_refetched
log "ok: registry came back for credentials after the reported expiration ($(credential_requests) requests)"

push_and_pull e2e/second

if "$tool" logs "$bridge" 2>&1 | grep -qF -e "$secret_key" -e "$(cat "$shared/token")"; then
  fail "imds-bridge logged a secret"
fi
log "PASS"

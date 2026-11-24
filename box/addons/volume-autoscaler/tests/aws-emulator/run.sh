#!/bin/bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: tests/aws-emulator/run.sh [cargo test filter and args]

Builds and starts AWS Emulator in a container, then runs the ignored
aws_emulator tests against it. Extra arguments go to the test binary.

Environment:
  CONTAINER_TOOL  docker or podman (default: whichever is installed, docker first)
  EMULATOR_PORT   host and container port (default: 9297)
  EMULATOR_IMAGE  image tag to build and run (default: volume-autoscaler-aws-emulator:local)
EOF
}

log() { printf '%s\n' "$*" >&2; }

case "${1:-}" in
  -h | --help)
    usage
    exit 0
    ;;
esac

here="$(cd "$(dirname "$0")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
port="${EMULATOR_PORT:-9297}"
image="${EMULATOR_IMAGE:-volume-autoscaler-aws-emulator:local}"

tool="${CONTAINER_TOOL:-}"
if [ -z "$tool" ]; then
  if command -v docker >/dev/null 2>&1; then
    tool=docker
  elif command -v podman >/dev/null 2>&1; then
    tool=podman
  else
    log "docker or podman is required"
    exit 1
  fi
fi
for cmd in "$tool" curl cargo; do
  command -v "$cmd" >/dev/null 2>&1 || { log "$cmd is required"; exit 1; }
done

case "$(uname -m)" in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *)
    log "unsupported architecture: $(uname -m)"
    exit 1
    ;;
esac

home="$(mktemp -d)"
name="volume-autoscaler-aws-emulator-$$"

cleanup() {
  "$tool" rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$home"
}
trap cleanup EXIT

log "building $image ($arch)"
"$tool" build --quiet --build-arg "ARCH=$arch" -t "$image" "$here" >/dev/null

user_args=()
if [ "$(uname -s)" = Linux ]; then
  user_args=(--user "$(id -u):$(id -g)")
fi

log "starting $name on 127.0.0.1:$port"
"$tool" run -d --name "$name" --memory=2g --memory-swap=2g \
  ${user_args[@]+"${user_args[@]}"} \
  -p "127.0.0.1:$port:$port" \
  -v "$home:/home/emulator/.aws/emulator" \
  "$image" -p "$port" --telemetry off >/dev/null

ca="$home/$port/ca.pem"
for _ in $(seq 60); do
  if [ -f "$ca" ] && curl -sf --cacert "$ca" "https://127.0.0.1:$port/__admin/health" >/dev/null; then
    break
  fi
  sleep 1
done
if ! curl -sf --cacert "$ca" "https://127.0.0.1:$port/__admin/health" >/dev/null; then
  "$tool" logs "$name" >&2 || true
  log "emulator did not become healthy"
  exit 1
fi

unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN AWS_PROFILE AWS_CONFIG_FILE
export AWS_ENDPOINT_URL="https://127.0.0.1:$port"
export AWS_SHARED_CREDENTIALS_FILE="$home/$port/emulator_credentials"
export SSL_CERT_FILE="$ca"
export AWS_EC2_METADATA_DISABLED=true
export AWS_REGION=us-east-1

cd "$crate"
cargo test --lib aws_emulator -- --ignored "$@"

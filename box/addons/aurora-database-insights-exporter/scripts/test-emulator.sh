#!/usr/bin/env bash
# Runs the ignored tests in tests/aws_emulator.rs against a throwaway AWS Emulator container.
#
# Usage: scripts/test-emulator.sh [extra cargo test args]
#   CONTAINER_ENGINE  docker or podman (default: whichever is installed, podman first)
#   EMULATOR_PORT     host port for the emulator (default: 9298)
#   KEEP_EMULATOR=1   leave the container running after the tests
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${EMULATOR_PORT:-9298}"
IMAGE="adie-aws-emulator:local"
NAME="adie-aws-emulator-${PORT}"

ENGINE="${CONTAINER_ENGINE:-}"
if [[ -z "$ENGINE" ]]; then
  if command -v podman >/dev/null 2>&1; then
    ENGINE=podman
  elif command -v docker >/dev/null 2>&1; then
    ENGINE=docker
  else
    echo "neither podman nor docker is installed" >&2
    exit 1
  fi
fi

case "$(uname -m)" in
  arm64 | aarch64) ARCH=aarch64 ;;
  *) ARCH=x86_64 ;;
esac

STATE="${ROOT}/target/aws-emulator-${PORT}"
rm -rf "$STATE" && mkdir -p "$STATE"
cleanup() {
  if [[ "${KEEP_EMULATOR:-}" != 1 ]]; then
    "$ENGINE" rm -f "$NAME" >/dev/null 2>&1 || true
    rm -rf "$STATE"
  fi
}
trap cleanup EXIT

"$ENGINE" build --quiet --build-arg "EMULATOR_ARCH=${ARCH}" -t "$IMAGE" "$ROOT/tests/aws-emulator" >/dev/null
"$ENGINE" rm -f "$NAME" >/dev/null 2>&1 || true
"$ENGINE" run -d --rm --name "$NAME" --memory=2g \
  -p "127.0.0.1:${PORT}:${PORT}" \
  -e HOME=/emulator -v "${STATE}:/emulator" \
  "$IMAGE" -p "$PORT" >/dev/null

FILES="${STATE}/.aws/emulator/${PORT}"
for _ in $(seq 1 60); do
  if [[ -f "${FILES}/ca.pem" ]] &&
    curl -fsS --cacert "${FILES}/ca.pem" "https://127.0.0.1:${PORT}/__admin/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS --cacert "${FILES}/ca.pem" "https://127.0.0.1:${PORT}/__admin/health" >/dev/null

unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN AWS_PROFILE AWS_DEFAULT_PROFILE AWS_ROLE_ARN AWS_WEB_IDENTITY_TOKEN_FILE
export AWS_ENDPOINT_URL="https://127.0.0.1:${PORT}"
export AWS_SHARED_CREDENTIALS_FILE="${FILES}/emulator_credentials"
export AWS_CONFIG_FILE="${STATE}/config"
export AWS_CA_BUNDLE="${FILES}/ca.pem"
export SSL_CERT_FILE="${FILES}/ca.pem"
export AWS_EC2_METADATA_DISABLED=true
export AWS_REGION=us-east-1
export AWS_DEFAULT_REGION=us-east-1
touch "$AWS_CONFIG_FILE"

cd "$ROOT"
cargo test --test aws_emulator "$@" -- --ignored

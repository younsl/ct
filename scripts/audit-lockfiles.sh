#!/usr/bin/env bash
#
# Fail when a dependency lockfile holds a known vulnerability.
# With no arguments every lockfile under box/addons is audited. With file
# arguments (pre-commit) only the lockfiles that own those files are.
#
#   ./scripts/audit-lockfiles.sh
#   ./scripts/audit-lockfiles.sh box/addons/forklift/Cargo.toml

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

LOCKFILES=(Cargo.lock package-lock.json pnpm-lock.yaml)

has_lockfile() {
  local name
  for name in "${LOCKFILES[@]}"; do
    if [[ -f "$1/$name" ]]; then return 0; fi
  done
  return 1
}

if [[ $# -eq 0 ]]; then
  dirs=$(find box/addons \( -name node_modules -o -name target \) -prune -o \
    \( -name Cargo.lock -o -name package-lock.json -o -name pnpm-lock.yaml \) -print |
    xargs -n1 dirname | sort -u)
else
  dirs=$(for f in "$@"; do
    d=$(dirname "$f")
    while [[ "$d" != "." && "$d" != "/" ]] && ! has_lockfile "$d"; do d=$(dirname "$d"); done
    if has_lockfile "$d"; then echo "$d"; fi
  done | sort -u)
fi

need() {
  if ! command -v "$1" > /dev/null; then
    echo "error: $1 is required to audit $2" >&2
    exit 2
  fi
}

# npm audit has no ignore list, so advisories listed in .npm-audit-ignore
# (one GHSA ID per line, # comments) are filtered out here.
npm_audit() {
  local ignored='[]' found
  if [[ -f .npm-audit-ignore ]]; then
    ignored=$(sed -e 's/#.*//' -e 's/[[:space:]]//g' -e '/^$/d' .npm-audit-ignore | jq -R . | jq -sc .)
  fi
  found=$(npm audit --json | jq -r --argjson ignored "$ignored" '
    [.vulnerabilities[]?.via[] | objects | {id: (.url | split("/") | last), name, severity, title}]
    | unique_by(.id)[]
    | select(.id as $id | $ignored | index($id) | not)
    | "\(.id) \(.name) (\(.severity)): \(.title)"') || true
  if [[ -n "$found" ]]; then
    echo "$found"
    return 1
  fi
}

# Vulnerabilities fail, and so does any other advisory a patched release fixes.
# Unmaintained and yanked notices have no patch and stay out.
cargo_audit() {
  local report found
  report=$(cargo audit --json "$@" 2> /dev/null) || true
  if ! jq -e '.vulnerabilities' <<< "$report" > /dev/null 2>&1; then
    echo "error: cargo audit produced no report" >&2
    return 1
  fi
  found=$(jq -r '
    (.vulnerabilities.list[] | . + {kind: "vulnerability"}),
    (.warnings[]?[] | select(.advisory != null and (.versions.patched | length > 0)))
    | "\(.advisory.id) \(.package.name) \(.package.version) (\(.kind), patched \(.versions.patched | join(" or ") | if . == "" then "none" else . end)): \(.advisory.title)"' <<< "$report")
  if [[ -n "$found" ]]; then
    echo "$found"
    return 1
  fi
}

failed=()
fetched=false

for d in $dirs; do
  if [[ -f "$d/Cargo.lock" ]]; then
    need cargo-audit "$d/Cargo.lock"
    need jq "$d/Cargo.lock"
    echo "==> $d: cargo audit"
    # The advisory database is fetched once per run. cd so .cargo/audit.toml applies.
    args=()
    if [[ "$fetched" == true ]]; then args+=(--no-fetch); fi
    fetched=true
    (cd "$d" && cargo_audit ${args[@]+"${args[@]}"}) || failed+=("$d/Cargo.lock")
  fi
  if [[ -f "$d/package-lock.json" ]]; then
    need npm "$d/package-lock.json"
    need jq "$d/package-lock.json"
    echo "==> $d: npm audit"
    (cd "$d" && npm_audit) || failed+=("$d/package-lock.json")
  fi
  if [[ -f "$d/pnpm-lock.yaml" ]]; then
    need pnpm "$d/pnpm-lock.yaml"
    echo "==> $d: pnpm audit"
    (cd "$d" && pnpm audit) || failed+=("$d/pnpm-lock.yaml")
  fi
done

if [[ ${#failed[@]} -gt 0 ]]; then
  echo
  echo "Vulnerable lockfiles:" >&2
  printf '  %s\n' "${failed[@]}" >&2
  exit 1
fi

#!/usr/bin/env bash
# ScoutChain — verify the code installed on-chain matches a release's WASM hashes (issue #1614)
#
# Usage:
#   ./scripts/verify-deployed-wasm.sh <network> <tag>
#
# Fetches the installed code of each contract ID in .env.contracts from the
# network and compares its sha256 (the on-chain code hash) with the
# `wasm-hashes.json` attached to GitHub Release <tag>. Reports MATCH/MISMATCH
# per contract and exits non-zero on any mismatch or fetch failure.
#
# Environment:
#   WASM_HASHES_FILE  use this local wasm-hashes.json instead of downloading it
#                     from the release (offline / pre-release verification)
#
# Prerequisites: stellar-cli, python3, sha256sum/shasum; `gh` unless WASM_HASHES_FILE is set.
set -euo pipefail

NETWORK="${1:?usage: verify-deployed-wasm.sh <network> <tag>}"
TAG="${2:?usage: verify-deployed-wasm.sh <network> <tag>}"

[[ -f .env.contracts ]] || { echo "ERROR: .env.contracts not found — did you run deploy.sh?" >&2; exit 1; }
# shellcheck source=/dev/null
source .env.contracts

if command -v sha256sum >/dev/null 2>&1; then
  hash_file() { sha256sum "$1" | awk '{print $1}'; }
else
  hash_file() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

HASHES="${WASM_HASHES_FILE:-}"
if [[ -z "$HASHES" ]]; then
  gh release download "$TAG" --pattern wasm-hashes.json --dir "$WORK"
  HASHES="$WORK/wasm-hashes.json"
fi

FAIL=0
for name in registration verification progress scout_access; do
  var="$(echo "$name" | tr '[:lower:]' '[:upper:]')_CONTRACT_ID"
  id="${!var:-}"
  if [[ -z "$id" ]]; then
    echo "FAIL  ${name}: ${var} is not set in .env.contracts"; FAIL=1; continue
  fi
  expected="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get(sys.argv[2],""))' "$HASHES" "$name")"
  if [[ -z "$expected" ]]; then
    echo "FAIL  ${name}: no expected hash for ${name} in release ${TAG}"; FAIL=1; continue
  fi
  if ! stellar contract fetch --id "$id" --network "$NETWORK" --out-file "$WORK/${name}.wasm" >/dev/null 2>&1; then
    echo "FAIL  ${name}: could not fetch installed code for ${id}"; FAIL=1; continue
  fi
  actual="$(hash_file "$WORK/${name}.wasm")"
  if [[ "$actual" == "$expected" ]]; then
    echo "MATCH     ${name} (${id}) ${actual}"
  else
    echo "MISMATCH  ${name} (${id}) on-chain=${actual} expected(${TAG})=${expected}"
    FAIL=1
  fi
done

if [[ "$FAIL" -ne 0 ]]; then
  echo "RESULT: deployed WASM does not match release ${TAG}" >&2
  exit 1
fi
echo "RESULT: all deployed contracts match release ${TAG}"

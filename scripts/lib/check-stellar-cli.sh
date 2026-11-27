#!/usr/bin/env bash
# Shared stellar-cli version check. Source this file, then call
#   check_stellar_cli_version <allow_mismatch: 0|1>
# The pinned version lives in ci/stellar-cli-version (single source of truth,
# also read by CI and generate-bindings.sh). A different CLI/optimiser version
# produces different WASM bytes and hashes than CI artifacts.

STELLAR_CLI_VERSION_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/ci/stellar-cli-version"
PINNED_STELLAR_CLI_VERSION="$(tr -d '[:space:]' < "$STELLAR_CLI_VERSION_FILE")"

check_stellar_cli_version() {
  local allow_mismatch="${1:-0}" actual
  actual=$(stellar --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)
  if [[ "$actual" == "$PINNED_STELLAR_CLI_VERSION" ]]; then
    return 0
  fi
  echo "stellar-cli version mismatch: pinned $PINNED_STELLAR_CLI_VERSION ($STELLAR_CLI_VERSION_FILE), found ${actual:-<not installed>}." >&2
  if [[ "$allow_mismatch" == "1" ]]; then
    echo "WARNING: continuing due to --allow-cli-mismatch; WASM hashes may not match CI artifacts." >&2
    return 0
  fi
  echo "ERROR: install stellar-cli $PINNED_STELLAR_CLI_VERSION or pass --allow-cli-mismatch." >&2
  exit 1
}

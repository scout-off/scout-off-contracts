#!/usr/bin/env bash
# ScoutChain — single source of truth for network selection.
# Source this file; it sets NETWORK (and NETWORK_PASSPHRASE) or exits on invalid input.
# Precedence: first script argument, then STELLAR_NETWORK, then "testnet".

NETWORK="${1:-${STELLAR_NETWORK:-testnet}}"

case "$NETWORK" in
  testnet) NETWORK_PASSPHRASE="Test SDF Network ; September 2015" ;;
  mainnet) NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015" ;;
  local)
    if [[ -z "${NETWORK_PASSPHRASE:-}" ]]; then
      echo "ERROR: STELLAR_NETWORK=local requires NETWORK_PASSPHRASE to be set." >&2
      exit 1
    fi
    ;;
  *)
    echo "ERROR: Unknown network '$NETWORK'. Expected exactly one of: testnet, mainnet, local." >&2
    exit 1
    ;;
esac

export NETWORK NETWORK_PASSPHRASE

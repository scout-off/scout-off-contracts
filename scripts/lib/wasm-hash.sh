#!/usr/bin/env bash
# Helpers for recording / reading on-chain WASM hashes (#1812). Source me.

# Print the sha256 of the WASM currently installed for <contract-id> on <network>.
onchain_wasm_hash() {
  local id="$1" network="$2" tmp
  tmp="$(mktemp)"
  if ! stellar contract fetch --id "$id" --network "$network" --out-file "$tmp" >/dev/null 2>&1; then
    rm -f "$tmp"; return 1
  fi
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$tmp" | awk '{print $1}'
  else shasum -a 256 "$tmp" | awk '{print $1}'; fi
  rm -f "$tmp"
}

# Set KEY=VALUE in an env file (replace existing line or append).
env_set() {
  local file="$1" key="$2" value="$3"
  touch "$file"
  if grep -q "^${key}=" "$file"; then
    sed -i.bak "s|^${key}=.*|${key}=${value}|" "$file" && rm -f "${file}.bak"
  else
    echo "${key}=${value}" >> "$file"
  fi
}

# Print the NAME (e.g. REGISTRATION) whose NAME_CONTRACT_ID equals <contract-id>.
contract_name_for_id() {
  local id="$1" file="${2:-.env.contracts}"
  [[ -f "$file" ]] || return 1
  grep -E "^[A-Z_]+_CONTRACT_ID=${id}\$" "$file" | head -1 | sed 's/_CONTRACT_ID=.*//'
}

# Mirror KEY=VALUE into .env.contracts.json when that file exists.
json_set() {
  local file="$1" key="$2" value="$3"
  [[ -f "$file" ]] || return 0
  python3 -I - "$file" "$key" "$value" <<'PY'
import json, sys
f, k, v = sys.argv[1:4]
d = json.load(open(f))
d[k] = v
json.dump(d, open(f, "w"), indent=2)
PY
}

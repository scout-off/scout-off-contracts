#!/usr/bin/env bash
# Reproducible WASM build (issue #1614). Intended to run inside ci/build/Dockerfile.
# Usage: ci/build/build-wasm.sh <out-dir>
#
# Emits, per contract, the optimized WASM plus `wasm-hashes.json`
# ({"<contract>": "<sha256>"}) and `SHA256SUMS`. Absolute paths are remapped so
# the checkout location does not leak into the binary.
set -euo pipefail

OUT="${1:?usage: build-wasm.sh <out-dir>}"
SRC="$(pwd)"
CONTRACTS=(registration verification progress scout_access)
WASM_DIR="target/wasm32v1-none/release"

export RUSTFLAGS="--remap-path-prefix=${SRC}=/src --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"

cargo build --workspace --target wasm32v1-none --release --locked

mkdir -p "$OUT"
: > "$OUT/SHA256SUMS"
json="{"
sep=""
for name in "${CONTRACTS[@]}"; do
  optimized="$OUT/scoutchain_${name}.optimized.wasm"
  stellar contract optimize --wasm "${WASM_DIR}/scoutchain_${name}.wasm" --wasm-out "$optimized"
  hash="$(sha256sum "$optimized" | awk '{print $1}')"
  echo "${hash}  scoutchain_${name}.optimized.wasm" >> "$OUT/SHA256SUMS"
  json+="${sep}\"${name}\":\"${hash}\""
  sep=","
done
echo "${json}}" > "$OUT/wasm-hashes.json"
cat "$OUT/SHA256SUMS"

#!/usr/bin/env bash
# Sandbox test: upgrade -> rollback --upgrade -> verify hash, using a fake
# `stellar` CLI that keeps contract code in a local directory (#1812).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT
mkdir "$W/bin" "$W/chain"; cd "$W"
ID=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
echo "old-wasm" > "$W/chain/$ID.wasm"; echo "new-wasm" > "$W/new.wasm"
cat > bin/stellar <<'FAKE'
#!/usr/bin/env bash
C="$(dirname "$0")/../chain"
case "$1 $2" in
  "contract fetch") id=""; out=""; while [[ $# -gt 0 ]]; do case "$1" in --id) id="$2"; shift;; --out-file) out="$2"; shift;; esac; shift; done; cp "$C/$id.wasm" "$out" ;;
  "contract upload") cp "$3" "$C/$(sha256sum "$3" | cut -d' ' -f1).blob"; sha256sum "$3" | cut -d' ' -f1 ;;
  "contract invoke") args="$*"; id="$(echo "$args" | sed -E 's/.*--id ([^ ]+).*/\1/')"
     if [[ "$args" == *"-- upgrade "* ]]; then h="${args##*-- upgrade }"; cp "$C/$h.blob" "$C/$id.wasm"; fi
     if [[ "$args" == *schema_version* ]]; then exit 1; fi ;;
  *) exit 0 ;;
esac
FAKE
chmod +x bin/stellar
export PATH="$W/bin:$PATH"
echo "REGISTRATION_CONTRACT_ID=$ID" > .env.contracts
old_hash="$(sha256sum "$W/chain/$ID.wasm" | cut -d' ' -f1)"
cp "$W/chain/$ID.wasm" "$W/chain/$old_hash.blob"
bash "$ROOT/scripts/upgrade.sh" --contract "$ID" --wasm new.wasm --source x --allow-cli-mismatch >/dev/null 2>&1 || true
grep -q "REGISTRATION_PREVIOUS_WASM_HASH=$old_hash" .env.contracts || { echo "FAIL: previous hash not recorded"; exit 1; }
bash "$ROOT/scripts/rollback-upgrade.sh" registration --source x
[[ "$(sha256sum "$W/chain/$ID.wasm" | cut -d' ' -f1)" == "$old_hash" ]] || { echo "FAIL: not reverted"; exit 1; }
[[ -f .env.contracts.pre-rollback ]] || { echo "FAIL: no backup"; exit 1; }
echo "rollback-upgrade test passed"

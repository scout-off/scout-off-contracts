#!/usr/bin/env bash
# Install the pinned stellar-cli release binary and verify its SHA-256.
# The version lives in ci/stellar-cli-version (single source of truth).
# Usage: ./scripts/install-stellar-cli.sh [install_dir]   (default: ~/.stellar/bin)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(tr -d '[:space:]' < "$ROOT/ci/stellar-cli-version")"
INSTALL_DIR="${1:-$HOME/.stellar/bin}"

# SHA-256 of the official release tarballs for $VERSION. Update together with
# ci/stellar-cli-version (see the release's asset digests on GitHub).
case "$VERSION/$(uname -m)" in
  28.0.0/x86_64)
    TARGET="x86_64-unknown-linux-gnu"
    SHA256="207544486734fccb4df1afc4a7745478f9f1e21688b2f9506f0ef36f60ce3fdc" ;;
  28.0.0/aarch64)
    TARGET="aarch64-unknown-linux-gnu"
    SHA256="8d8e5ea9bb441a19e99a7a7a4809df3f758c9654265c76953436fb7e9288b9ff" ;;
  *)
    echo "ERROR: no pinned checksum for stellar-cli $VERSION on $(uname -m)" >&2
    exit 1 ;;
esac

TARBALL="stellar-cli-${VERSION}-${TARGET}.tar.gz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL -o "$TMP/$TARBALL" \
  "https://github.com/stellar/stellar-cli/releases/download/v${VERSION}/${TARBALL}"
echo "${SHA256}  $TMP/$TARBALL" | sha256sum -c -
tar -xzf "$TMP/$TARBALL" -C "$TMP" stellar
mkdir -p "$INSTALL_DIR"
install -m 0755 "$TMP/stellar" "$INSTALL_DIR/stellar"
"$INSTALL_DIR/stellar" --version

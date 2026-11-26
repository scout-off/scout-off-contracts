#!/usr/bin/env bash
# Stop and remove the local stellar/quickstart sandbox started by setup-local.sh.
set -euo pipefail
CONTAINER="${LOCAL_CONTAINER:-stellar-local}"
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
echo "Removed container $CONTAINER (if it existed)."

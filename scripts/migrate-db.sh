#!/usr/bin/env bash
# Apply migrations/NNN_*.sql in order, idempotently (#1810).
# Records each applied file in schema_migrations(version, checksum, applied_at)
# and refuses to run if an already-applied file's checksum has changed.
# Usage: DATABASE_URL=postgres://... ./scripts/migrate-db.sh [migrations-dir]
set -euo pipefail

DIR="${1:-$(cd "$(dirname "$0")/.." && pwd)/migrations}"
: "${DATABASE_URL:?DATABASE_URL is required}"

PSQL=(psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -X -q -t -A)

"${PSQL[@]}" -c "CREATE TABLE IF NOT EXISTS schema_migrations (
  version    TEXT PRIMARY KEY,
  checksum   TEXT NOT NULL,
  applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);"

applied=0
shopt -s nullglob
for f in "$DIR"/[0-9][0-9][0-9]_*.sql; do
  version="$(basename "$f" .sql)"
  checksum="$(sha256sum "$f" | cut -d' ' -f1)"
  recorded="$("${PSQL[@]}" -c "SELECT checksum FROM schema_migrations WHERE version = '$version'")"
  if [[ -n "$recorded" ]]; then
    if [[ "$recorded" != "$checksum" ]]; then
      echo "ERROR: applied migration $version was modified (recorded $recorded, now $checksum)" >&2
      exit 1
    fi
    echo "skip   $version"
    continue
  fi
  echo "apply  $version"
  {
    echo "BEGIN;"
    cat "$f"
    echo
    echo "INSERT INTO schema_migrations (version, checksum) VALUES ('$version', '$checksum');"
    echo "COMMIT;"
  } | "${PSQL[@]}" -f - >/dev/null
  applied=$((applied + 1))
done
echo "Done: $applied migration(s) applied."

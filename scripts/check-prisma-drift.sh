#!/usr/bin/env bash
# check-prisma-drift.sh
#
# Fails if prisma/schema.prisma has drifted from the SQL migrations, which are
# canonical (see migrations/README.md). Applies migrations/*.sql in order to
# the database at $DATABASE_URL (must be empty), then diffs the migrated
# database against the Prisma datamodel.
#
# Only tables that have a Prisma model are compared: the SQL schema has many
# indexer tables with no model, so drops of those are expected and ignored.
#
# Usage: DATABASE_URL=postgresql://... bash scripts/check-prisma-drift.sh

set -euo pipefail

: "${DATABASE_URL:?DATABASE_URL must point at an empty Postgres database}"

cd "$(dirname "${BASH_SOURCE[0]}")/.."

for f in migrations/*.sql; do
  echo "Applying $f"
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -q -f "$f" >/dev/null
done

# Tables declared via @@map in the Prisma schema.
mapfile -t TABLES < <(sed -n 's/^[[:space:]]*@@map("\([^"]*\)").*/\1/p' prisma/schema.prisma)
if [[ ${#TABLES[@]} -eq 0 ]]; then
  echo "No @@map entries found in prisma/schema.prisma" >&2
  exit 1
fi

DIFF="$(npx prisma migrate diff \
  --from-url "$DATABASE_URL" \
  --to-schema-datamodel prisma/schema.prisma \
  --script)"

# Keep only the statements that touch a modeled table, and drop the DROP TABLE
# statements for unmodeled tables.
DRIFT="$(printf '%s\n' "$DIFF" | python3 -c '
import re, sys
tables = set(sys.argv[1:])
text = sys.stdin.read()
out = []
for stmt in re.split(r";\s*\n", text):
    body = "\n".join(l for l in stmt.splitlines() if not l.strip().startswith("--")).strip()
    if not body:
        continue
    named = set(re.findall(r"\"([A-Za-z0-9_]+)\"", body))
    if named & tables:
        out.append(body + ";")
print("\n\n".join(out))
' "${TABLES[@]}")"

if [[ -n "$DRIFT" ]]; then
  echo "ERROR: prisma/schema.prisma has drifted from migrations/*.sql." >&2
  echo "Update the Prisma schema to match the SQL (or add the missing migration)." >&2
  echo "SQL needed to turn the migrated database into the Prisma schema:" >&2
  echo "$DRIFT" >&2
  exit 1
fi

echo "OK: Prisma schema matches migrations for: ${TABLES[*]}"

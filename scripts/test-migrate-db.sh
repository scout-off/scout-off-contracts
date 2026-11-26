#!/usr/bin/env bash
# Idempotency / tamper-detection test for scripts/migrate-db.sh (#1810).
# Requires DATABASE_URL pointing at an empty Postgres database.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${DATABASE_URL:?DATABASE_URL is required}"

expected=$(ls migrations/[0-9][0-9][0-9]_*.sql | wc -l)
bash scripts/migrate-db.sh
out=$(bash scripts/migrate-db.sh)
echo "$out" | grep -q "0 migration(s) applied" || { echo "FAIL: second run not idempotent"; exit 1; }
count=$(psql "$DATABASE_URL" -X -t -A -c "SELECT count(*) FROM schema_migrations")
[[ "$count" == "$expected" ]] || { echo "FAIL: schema_migrations has $count rows, expected $expected"; exit 1; }

psql "$DATABASE_URL" -X -q -c "UPDATE schema_migrations SET checksum='tampered' WHERE version='001_initial_schema'"
if bash scripts/migrate-db.sh 2>/dev/null; then echo "FAIL: modified migration not detected"; exit 1; fi
# restore the real checksum so later steps see a consistent state
sum=$(sha256sum migrations/001_initial_schema.sql | cut -d' ' -f1)
psql "$DATABASE_URL" -X -q -c "UPDATE schema_migrations SET checksum='$sum' WHERE version='001_initial_schema'"
echo "migrate-db tests passed"

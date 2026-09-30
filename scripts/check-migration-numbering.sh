#!/usr/bin/env bash
# check-migration-numbering.sh � CI lint step enforcing unique, contiguous numeric
# prefixes for database migration files in migrations/.
#
# Asserts:
#   1. All SQL migration files match the naming convention: ^[0-9]{3}_[a-z0-9_]+\.sql$
#   2. Numeric prefixes are unique (no two migrations share the same number).
#   3. Numeric sequence starts at 001 and is strictly contiguous with no gaps (001, 002, ..., N).
#
# Exit codes:
#   0 � all migration files are valid, unique, and strictly contiguous
#   1 � one or more naming, duplicate, or gap violations found

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MIGRATIONS_DIR="$REPO_ROOT/migrations"

FAIL=0

if [[ ! -d "$MIGRATIONS_DIR" ]]; then
  echo "ERROR: migrations directory not found at $MIGRATIONS_DIR"
  exit 1
fi

echo "Checking migration file numbering in migrations/..."

# Collect all .sql files in migrations/
shopt -s nullglob
files=("$MIGRATIONS_DIR"/*.sql)
shopt -u nullglob

if [[ ${#files[@]} -eq 0 ]]; then
  echo "ERROR: No .sql migration files found in $MIGRATIONS_DIR"
  exit 1
fi

declare -A seen_numbers
numbers=()

for filepath in "${files[@]}"; do
  filename="$(basename "$filepath")"

  if [[ ! "$filename" =~ ^([0-9]{3})_[a-z0-9_]+\.sql$ ]]; then
    echo "FAIL: Migration filename '$filename' does not match required format 'NNN_<name>.sql' (e.g. 001_initial_schema.sql)"
    FAIL=1
    continue
  fi

  prefix="${BASH_REMATCH[1]}"
  num=$((10#$prefix))

  if [[ -n "${seen_numbers[$prefix]:-}" ]]; then
    echo "FAIL: Duplicate migration prefix '$prefix': '$filename' collides with '${seen_numbers[$prefix]}'"
    FAIL=1
  else
    seen_numbers[$prefix]="$filename"
    numbers+=("$num")
  fi
done

expected=1
for num in $(printf "%s\n" "${numbers[@]}" | sort -n); do
  if [[ $num -ne $expected ]]; then
    printf "FAIL: Migration sequence gap detected! Expected %03d, but found %03d\n" "$expected" "$num"
    FAIL=1
  fi
  expected=$((num + 1))
done

if [[ $FAIL -ne 0 ]]; then
  echo ""
  echo "Migration numbering check FAILED. Please ensure every file in migrations/ has a unique, contiguous 3-digit prefix (001, 002, 003, ...)."
  exit 1
fi

last_num=$((expected - 1))
printf "OK: %d migration files verified with unique, contiguous numbering (001 through %03d).\n" "${#files[@]}" "$last_num"
exit 0

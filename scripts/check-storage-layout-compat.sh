#!/usr/bin/env bash
# check-storage-layout-compat.sh
#
# Compares DataKey enums and every #[contracttype] struct/enum between two
# Rust source references and reports whether any storage-layout breaking
# change has been introduced, per the rules in docs/VERSIONING.md.
#
# Usage:
#   bash scripts/check-storage-layout-compat.sh <old-ref> <new-ref> [--acknowledge-breaking-change]
#
#   <old-ref> and <new-ref> may be:
#     - A path to a single types.rs file
#     - A path to the repo root directory (contracts/ subdirectory is used)
#     - A git ref (commit SHA, branch name, or tag)
#
# Exit codes:
#   0 — no breaking changes (or all acknowledged with the flag)
#   1 — one or more breaking changes detected; upgrade blocked
#   2 — usage error

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
if [[ $# -lt 2 ]]; then
  echo "Usage: $0 <old-ref> <new-ref> [--acknowledge-breaking-change]"
  echo "  <old-ref> and <new-ref> may be git refs or paths to types.rs files."
  exit 2
fi

OLD_REF="$1"
NEW_REF="$2"
ACKNOWLEDGE=0
if [[ "${3:-}" == "--acknowledge-breaking-change" ]]; then
  ACKNOWLEDGE=1
fi

BREAKING=0
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

# Write the Python extractor to a temp file
EXTRACTOR_PY="${WORK_DIR}/extractor.py"
cat > "$EXTRACTOR_PY" << 'PYEOF'
#!/usr/bin/env python3
"""
Extracts all #[contracttype] items from a Rust source file and prints them
as a structured text summary suitable for diffing.

Output format per item:
  TYPE <kind> <name>
  FIELD <name> <type>   (for structs)
  VARIANT <name>        (for enums)
  END
"""
import re
import sys


def extract_contracttype_items(src):
    # Remove block comments
    src = re.sub(r'/\*.*?\*/', '', src, flags=re.DOTALL)
    # Remove line comments
    src = re.sub(r'//[^\n]*', '', src)

    items = []
    i = 0
    pattern = re.compile(r'#\[contracttype\]')

    while True:
        m = pattern.search(src, i)
        if not m:
            break
        i = m.end()

        # Find the next struct or enum keyword within 2000 chars
        segment = src[i:i + 2000]
        kind_m = re.search(r'\b(pub\s+)?(struct|enum)\s+(\w+)', segment)
        if not kind_m:
            continue

        kind = kind_m.group(2)
        name = kind_m.group(3)

        # Collect the body between the first { and matching }
        body_start = segment.find('{', kind_m.end())
        if body_start == -1:
            continue

        depth = 0
        body_chars = []
        for ch in segment[body_start:]:
            body_chars.append(ch)
            if ch == '{':
                depth += 1
            elif ch == '}':
                depth -= 1
                if depth == 0:
                    break

        body = ''.join(body_chars)
        items.append((kind, name, body))

    return items


def parse_struct_fields(body):
    """Returns list of (field_name, field_type) pairs."""
    fields = []
    for m in re.finditer(r'\bpub\s+(\w+)\s*:\s*([^,\n}]+)', body):
        fname = m.group(1).strip()
        ftype = re.sub(r'\s+', ' ', m.group(2).strip().rstrip(','))
        fields.append((fname, ftype))
    return fields


def parse_enum_variants(body):
    """Returns list of variant names in order."""
    variants = []
    seen = set()
    for m in re.finditer(r'^\s*([A-Z][A-Za-z0-9_]*)(?:\s*[,({\n])', body, re.MULTILINE):
        v = m.group(1)
        if v not in seen:
            seen.add(v)
            variants.append(v)
    return variants


def summarize_file(path):
    try:
        src = open(path).read()
    except FileNotFoundError:
        return []

    items = extract_contracttype_items(src)
    lines = []
    for kind, name, body in items:
        lines.append(f"TYPE {kind} {name}")
        if kind == 'struct':
            for fname, ftype in parse_struct_fields(body):
                lines.append(f"FIELD {fname} {ftype}")
        else:
            for vname in parse_enum_variants(body):
                lines.append(f"VARIANT {vname}")
        lines.append("END")
    return lines


if __name__ == '__main__':
    for line in summarize_file(sys.argv[1]):
        print(line)
PYEOF

# Write the Python comparator to a temp file
COMPARATOR_PY="${WORK_DIR}/comparator.py"
cat > "$COMPARATOR_PY" << 'PYEOF'
#!/usr/bin/env python3
"""
Compares two contracttype summary files and classifies every difference
as SAFE or BREAKING per docs/VERSIONING.md.
"""
import os
import sys


def parse_summary(path):
    """Returns dict: name -> {'kind': str, 'fields': list, 'variants': list}"""
    items = {}
    current = None
    try:
        lines = open(path).readlines()
    except FileNotFoundError:
        return {}

    for line in lines:
        line = line.strip()
        if not line:
            continue
        if line.startswith('TYPE '):
            parts = line.split(None, 2)
            kind, name = parts[1], parts[2]
            current = {'kind': kind, 'fields': [], 'variants': []}
            items[name] = current
        elif line.startswith('FIELD ') and current is not None:
            parts = line.split(None, 2)
            fname = parts[1]
            ftype = parts[2] if len(parts) > 2 else ''
            current['fields'].append((fname, ftype))
        elif line.startswith('VARIANT ') and current is not None:
            vname = line.split(None, 1)[1]
            current['variants'].append(vname)
        elif line == 'END':
            current = None
    return items


def compare_contract(old_path, new_path, contract):
    old = parse_summary(old_path)
    new = parse_summary(new_path)

    breaking = []
    safe_changes = []

    all_names = sorted(set(list(old.keys()) + list(new.keys())))
    for name in all_names:
        if name not in old and name in new:
            safe_changes.append(f"  SAFE     [{contract}] {name}: new type added")
            continue
        if name in old and name not in new:
            breaking.append(f"  BREAKING [{contract}] {name}: type removed")
            continue

        o = old[name]
        n = new[name]

        if o['kind'] != n['kind']:
            breaking.append(
                f"  BREAKING [{contract}] {name}: kind changed "
                f"from {o['kind']} to {n['kind']}"
            )
            continue

        if o['kind'] == 'struct':
            old_fields = o['fields']
            new_fields = n['fields']
            if old_fields == new_fields:
                continue

            for i, (fname, ftype) in enumerate(old_fields):
                if i >= len(new_fields):
                    breaking.append(
                        f"  BREAKING [{contract}] {name}.{fname}: field removed"
                    )
                elif new_fields[i][0] != fname:
                    breaking.append(
                        f"  BREAKING [{contract}] {name}.{fname}: "
                        f"field reordered or renamed at position {i} "
                        f"(was '{fname}', now '{new_fields[i][0]}')"
                    )
                elif new_fields[i][1] != ftype:
                    breaking.append(
                        f"  BREAKING [{contract}] {name}.{fname}: "
                        f"type changed from '{ftype}' to '{new_fields[i][1]}'"
                    )
            for i in range(len(old_fields), len(new_fields)):
                safe_changes.append(
                    f"  SAFE     [{contract}] {name}.{new_fields[i][0]}: "
                    f"new field appended at end"
                )

        else:  # enum
            old_variants = o['variants']
            new_variants = n['variants']
            if old_variants == new_variants:
                continue

            old_set = set(old_variants)
            new_set = set(new_variants)

            for v in old_set - new_set:
                breaking.append(f"  BREAKING [{contract}] {name}::{v}: variant removed")

            for v in new_set - old_set:
                new_idx = new_variants.index(v)
                # Safe only if appended after all old variants
                if new_idx >= len(old_variants):
                    safe_changes.append(
                        f"  SAFE     [{contract}] {name}::{v}: "
                        f"new variant appended at end"
                    )
                else:
                    breaking.append(
                        f"  BREAKING [{contract}] {name}::{v}: "
                        f"new variant inserted before existing variants "
                        f"(changes discriminant mapping)"
                    )

            # Check reordering of retained variants
            retained_old = [v for v in old_variants if v in new_set]
            retained_new = [v for v in new_variants if v in old_set]
            if retained_old != retained_new:
                breaking.append(
                    f"  BREAKING [{contract}] {name}: "
                    f"existing variants reordered (discriminant mapping changed)"
                )

    return breaking, safe_changes


def main():
    old_dir = sys.argv[1]
    new_dir = sys.argv[2]

    contracts = ['registration', 'verification', 'progress', 'scout_access']
    any_breaking = False
    any_output = False

    for contract in contracts:
        old_sum = os.path.join(old_dir, f'{contract}_summary.txt')
        new_sum = os.path.join(new_dir, f'{contract}_summary.txt')
        breaking, safe_changes = compare_contract(old_sum, new_sum, contract)
        if breaking or safe_changes:
            any_output = True
            for msg in safe_changes:
                print(msg)
            for msg in breaking:
                print(msg)
                any_breaking = True

    if not any_output:
        print("  (no contracttype changes detected)")

    sys.exit(1 if any_breaking else 0)


if __name__ == '__main__':
    main()
PYEOF

# ---------------------------------------------------------------------------
# Helpers: resolve a ref or path to a directory containing types.rs files
# ---------------------------------------------------------------------------

is_file_path() {
  [[ -f "$1" || -d "$1" ]]
}

# Extract types.rs files for all four contracts from a git ref into dest dir.
extract_git_ref() {
  local ref="$1"
  local dest="$2"
  mkdir -p "$dest"
  for contract in registration verification progress scout_access; do
    local src_path="contracts/${contract}/src/types.rs"
    if git -C "$REPO_ROOT" show "${ref}:${src_path}" \
        > "${dest}/${contract}_types.rs" 2>/dev/null; then
      :
    else
      # Contract did not exist at this ref — produce empty file
      : > "${dest}/${contract}_types.rs"
    fi
  done
}

# Copy types.rs files from a directory or a single file into dest dir.
copy_from_path() {
  local src="$1"
  local dest="$2"
  mkdir -p "$dest"
  if [[ -f "$src" ]]; then
    # Single file — treat as the types.rs for all contracts (fixture mode)
    for contract in registration verification progress scout_access; do
      cp "$src" "${dest}/${contract}_types.rs"
    done
  else
    for contract in registration verification progress scout_access; do
      local f="${src}/contracts/${contract}/src/types.rs"
      if [[ -f "$f" ]]; then
        cp "$f" "${dest}/${contract}_types.rs"
      else
        : > "${dest}/${contract}_types.rs"
      fi
    done
  fi
}

# ---------------------------------------------------------------------------
# Resolve old and new source directories
# ---------------------------------------------------------------------------

OLD_SRC="${WORK_DIR}/old_src"
NEW_SRC="${WORK_DIR}/new_src"

if is_file_path "$OLD_REF"; then
  copy_from_path "$OLD_REF" "$OLD_SRC"
else
  extract_git_ref "$OLD_REF" "$OLD_SRC"
fi

if is_file_path "$NEW_REF"; then
  copy_from_path "$NEW_REF" "$NEW_SRC"
else
  extract_git_ref "$NEW_REF" "$NEW_SRC"
fi

# ---------------------------------------------------------------------------
# Step 1: Extract summaries
# ---------------------------------------------------------------------------

OLD_SUM="${WORK_DIR}/old_sum"
NEW_SUM="${WORK_DIR}/new_sum"
mkdir -p "$OLD_SUM" "$NEW_SUM"

for contract in registration verification progress scout_access; do
  python3 "$EXTRACTOR_PY" "${OLD_SRC}/${contract}_types.rs" \
    > "${OLD_SUM}/${contract}_summary.txt" 2>/dev/null || \
    : > "${OLD_SUM}/${contract}_summary.txt"

  python3 "$EXTRACTOR_PY" "${NEW_SRC}/${contract}_types.rs" \
    > "${NEW_SUM}/${contract}_summary.txt" 2>/dev/null || \
    : > "${NEW_SUM}/${contract}_summary.txt"
done

# ---------------------------------------------------------------------------
# Step 2: Compare
# ---------------------------------------------------------------------------

echo "=== Storage Layout Compatibility Check ==="
echo "  old: ${OLD_REF}"
echo "  new: ${NEW_REF}"
echo ""

COMPARE_OUT="${WORK_DIR}/compare_result.txt"
if python3 "$COMPARATOR_PY" "$OLD_SUM" "$NEW_SUM" > "$COMPARE_OUT" 2>&1; then
  :
else
  BREAKING=1
fi

cat "$COMPARE_OUT"

# ---------------------------------------------------------------------------
# Step 3: Report
# ---------------------------------------------------------------------------

if grep -q "^  BREAKING" "$COMPARE_OUT" 2>/dev/null; then
  BREAKING=1
fi

echo ""
if [[ $BREAKING -ne 0 ]]; then
  echo "RESULT: BREAKING storage-layout changes detected."
  if [[ $ACKNOWLEDGE -eq 1 ]]; then
    echo ""
    echo "  --acknowledge-breaking-change flag provided."
    echo "  Continuing with upgrade. Ensure a data migration or storage drain"
    echo "  has been prepared before this WASM is activated on a live network."
    echo ""
    exit 0
  else
    echo ""
    echo "  Upgrade blocked. Either:"
    echo "    1. Revise the change to avoid a breaking storage-layout modification."
    echo "    2. Prepare a data migration and re-run with --acknowledge-breaking-change."
    echo ""
    echo "  See docs/VERSIONING.md for the full classification rules."
    echo ""
    exit 1
  fi
else
  echo "RESULT: No breaking storage-layout changes detected."
  echo ""
  exit 0
fi

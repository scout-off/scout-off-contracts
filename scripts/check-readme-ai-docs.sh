#!/usr/bin/env bash
# check-readme-ai-docs.sh — verify README.md and ai.md mention every public
# contract entrypoint (`pub fn` inside #[contractimpl] blocks).
#
# Functions that are intentionally undocumented go in docs/.undocumented-allowlist
# (format: `<contract>::<fn>  # reason`, or `<fn>  # reason` for any contract).
#
# Warn-only by default (exit 0); pass --strict (or STRICT=1) to exit 1 on
# missing entries. Flip CI to --strict once the tracked doc gaps are closed.
# No cargo build required.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STRICT="${STRICT:-0}"
[[ "${1:-}" == "--strict" ]] && STRICT=1

python3 - "$REPO_ROOT" "$STRICT" <<'PYEOF'
import os, re, sys

root, strict = sys.argv[1], sys.argv[2] == "1"
contracts = ["registration", "verification", "progress", "scout_access"]
docs = ["README.md", "ai.md"]

allow = set()
allow_path = os.path.join(root, "docs", ".undocumented-allowlist")
if os.path.exists(allow_path):
    for line in open(allow_path):
        entry = line.split("#", 1)[0].strip()
        if entry:
            allow.add(entry)

def pub_fns(path):
    src = open(path).read()
    m = re.search(r'^#\[cfg\(test\)\]\s*\nmod \w+', src, re.M)
    if m:
        src = src[:m.start()]
    names = []
    for seg in re.split(r'#\[contractimpl\]', src)[1:]:
        if not seg.lstrip().startswith("impl"):
            continue
        depth, started, buf = 0, False, []
        for ch in seg:
            if ch == "{":
                depth += 1
                started = True
            elif ch == "}":
                depth -= 1
            buf.append(ch)
            if started and depth == 0:
                break
        body, depth, line = "".join(buf), 0, ""
        for ch in body:
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
            elif depth == 1:
                line += ch
        # top-level of the impl body only (depth 1 chars)
        names += re.findall(r'\bpub fn (\w+)', line)
    return names

texts = {d: open(os.path.join(root, d)).read() for d in docs if os.path.exists(os.path.join(root, d))}
missing = {}
for c in contracts:
    p = os.path.join(root, "contracts", c, "src", "lib.rs")
    if not os.path.exists(p):
        continue
    for fn in sorted(set(pub_fns(p))):
        if fn in allow or f"{c}::{fn}" in allow:
            continue
        for d, t in texts.items():
            if not re.search(r'\b' + re.escape(fn) + r'\b', t):
                missing.setdefault((c, d), []).append(fn)

if not missing:
    print("check-readme-ai-docs: OK — all public entrypoints are documented")
    sys.exit(0)

label = "ERROR" if strict else "WARNING"
for (c, d), fns in sorted(missing.items()):
    print(f"{label}: {c}: missing from {d}: {', '.join(fns)}")
print("Add them to the docs or to docs/.undocumented-allowlist with a reason.")
sys.exit(1 if strict else 0)
PYEOF

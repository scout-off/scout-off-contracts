#!/usr/bin/env python3
"""Normalise a generated bindings/<name>/package.json (ESM-only package).

`stellar contract bindings typescript --overwrite` regenerates package.json from
the CLI template, so generate-bindings.sh runs this after every generation.
It makes the package correct ESM-only (tsc emits only ES modules):
  - "type": "module", `exports` without the never-built dist/index.cjs
  - @stellar/stellar-sdk as a peerDependency + devDependency only
  - real `test` (type-check) and `test:integration` scripts, publint/attw checks
"""
import json
import sys

SDK = "@stellar/stellar-sdk"
path = sys.argv[1]
with open(path) as f:
    d = json.load(f)

SDK_RANGE = d.get("peerDependencies", {}).get(SDK) or d.get("dependencies", {}).get(SDK) or "^13.0.0"

d["type"] = "module"
d["main"] = "dist/index.js"
d["types"] = "dist/index.d.ts"
d["exports"] = {".": {"types": "./dist/index.d.ts", "import": "./dist/index.js", "default": "./dist/index.js"}}
d.get("dependencies", {}).pop(SDK, None)
if not d.get("dependencies"):
    d.pop("dependencies", None)
d.setdefault("peerDependencies", {})[SDK] = SDK_RANGE
d.setdefault("devDependencies", {})[SDK] = SDK_RANGE
d["devDependencies"]["vitest"] = "^2.1.0"
d.setdefault("scripts", {})
d["scripts"]["test"] = "tsc --noEmit -p tsconfig.json"
d["scripts"]["test:integration"] = "vitest run examples"
d["scripts"]["check:package"] = "publint && attw --pack . --profile esm-only"

with open(path, "w") as f:
    json.dump(d, f, indent=2, ensure_ascii=False)
    f.write("\n")

// Smoke test: every bindings package must be importable as ESM (they are
// ESM-only; `require` is intentionally not exported) and expose `Client`.
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { resolve } from "node:path";

const require = createRequire(import.meta.url);
let failed = false;
for (const pkg of ["registration", "verification", "progress", "scout_access"]) {
  const dir = resolve("bindings", pkg);
  const manifest = require(resolve(dir, "package.json"));
  if (manifest.type !== "module") throw new Error(`${pkg}: "type" must be "module"`);
  if (manifest.dependencies?.["@stellar/stellar-sdk"]) throw new Error(`${pkg}: SDK must be a peer dependency only`);
  try {
    const mod = await import(pathToFileURL(resolve(dir, "dist/index.js")).href);
    if (!mod.Client) throw new Error("no Client export");
    console.log(`OK: ${manifest.name} imports as ESM`);
  } catch (e) {
    console.error(`FAIL: ${manifest.name}: ${e.message}`);
    failed = true;
  }
}
process.exit(failed ? 1 : 0);

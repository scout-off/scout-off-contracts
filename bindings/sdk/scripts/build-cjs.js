#!/usr/bin/env node
// Post-build: copy ESM dist/*.js to dist/*.cjs for CommonJS consumers.
// This keeps the tsconfig outputting ESM (clean types) while still shipping
// a CJS build that works with require().
"use strict";
const fs = require("fs");
const path = require("path");

const DIST = path.join(__dirname, "../dist");

function walk(dir) {
  const entries = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) entries.push(...walk(full));
    else entries.push(full);
  }
  return entries;
}

if (!fs.existsSync(DIST)) {
  console.warn("dist/ not found, skipping CJS build");
  process.exit(0);
}

let count = 0;
for (const file of walk(DIST)) {
  if (!file.endsWith(".js")) continue;
  // Rewrite import/export specifiers ending in .js → .cjs for the CJS copy.
  const src = fs.readFileSync(file, "utf8");
  const cjs = src
    .replace(/from\s+"(\..*?)\.js"/g, 'from "$1.cjs"')
    .replace(/export \* from\s+"(\..*?)\.js"/g, 'export * from "$1.cjs"')
    .replace(/import\("(\..*?)\.js"\)/g, 'import("$1.cjs")');
  const dest = file.replace(/\.js$/, ".cjs");
  fs.writeFileSync(dest, cjs);
  count++;
}
console.log(`CJS build: wrote ${count} .cjs files to dist/`);

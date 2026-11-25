"use strict";
/** @type {import('jest').Config} */
module.exports = {
  testEnvironment: "node",
  transform: {},
  extensionsToTreatAsEsm: [".ts"],
  testMatch: ["**/tests/**/*.test.ts", "**/tests/**/*.test.js"],
  moduleFileExtensions: ["js", "mjs", "ts"],
};

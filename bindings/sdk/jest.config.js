"use strict";
/** @type {import('jest').Config} */
module.exports = {
  testEnvironment: "node",
  preset: "ts-jest",
  transform: {
    "^.+\\.tsx?$": [
      "ts-jest",
      {
        tsconfig: {
          module: "CommonJS",
          moduleResolution: "node",
        },
      },
    ],
  },
  testMatch: ["**/tests/**/*.test.ts", "**/tests/**/*.test.js"],
  moduleFileExtensions: ["ts", "js", "mjs"],
};

// Flat config (#1802). Type-aware rules catch unhandled-async-route bugs.
const js = require("@eslint/js");
const tseslint = require("typescript-eslint");

module.exports = tseslint.config(
  { ignores: ["dist/**", "bindings/**", "node_modules/**", "scripts/**", "eslint.config.js", "jest.config.js"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.ts", "tests/**/*.ts"],
    languageOptions: {
      parserOptions: { project: "./tsconfig.json", tsconfigRootDir: __dirname }
    },
    rules: {
      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-misused-promises": "error",
      "no-console": "error"
    }
  },
  {
    files: ["src/server.ts", "tests/**/*.ts"],
    rules: { "no-console": "off" }
  }
);

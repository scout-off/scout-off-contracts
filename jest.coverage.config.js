// Coverage run config (#1805). Extends the base jest config and adds
// collectCoverageFrom plus a non-decreasing threshold floor. Raise these
// numbers as coverage improves; never lower them. See docs/COVERAGE.md.
const base = require("./jest.config.js");

module.exports = {
  ...base,
  collectCoverageFrom: ["src/**/*.ts", "!src/**/*.d.ts"],
  coverageDirectory: "coverage",
  coverageThreshold: {
    global: {
      statements: 10,
      branches: 5,
      functions: 10,
      lines: 10
    }
  }
};

const base = {
  preset: "ts-jest",
  testEnvironment: "node",
  roots: ["<rootDir>/tests", "<rootDir>/src"],
  setupFiles: ["<rootDir>/tests/setupEnv.ts"],
  transform: {
    "^.+\\.tsx?$": "ts-jest"
  }
};

// Unit tests mock Prisma; integration tests (*.int.test.ts) need a live
// Postgres via DATABASE_URL and skip themselves when it is absent (#1803).
module.exports = {
  projects: [
    { ...base, displayName: "unit", testPathIgnorePatterns: ["/node_modules/", "\\.int\\.test\\.ts$"] },
    { ...base, displayName: "integration", testMatch: ["**/*.int.test.ts"] }
  ]
};

module.exports = {
  preset: "ts-jest",
  testEnvironment: "node",
  testMatch: ["**/*.test.ts"],
  roots: ["<rootDir>/tests"],
  transform: {
    "^.+\\.tsx?$": "ts-jest"
  },
  globals: {
    "ts-jest": {
      tsconfig: {
        strict: true,
        esModuleInterop: true,
        moduleResolution: "node",
        skipLibCheck: true,
      }
    }
  },
  testPathIgnorePatterns: [
    "/node_modules/",
    "<rootDir>/e2e/"
  ]
};

module.exports = {
  preset: "ts-jest",
  testEnvironment: "node",
  roots: ["<rootDir>/tests"],
  setupFiles: ["<rootDir>/tests/setupEnv.ts"],
  transform: {
    "^.+\\.tsx?$": "ts-jest"
  }
};

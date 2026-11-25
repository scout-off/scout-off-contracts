module.exports = {
  preset: "ts-jest",
  testEnvironment: "node",
  roots: ["<rootDir>/tests", "<rootDir>/src", "<rootDir>/bindings/sdk/tests"],
  setupFiles: ["<rootDir>/tests/setupEnv.ts"],
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
  }
};

/**
 * Jest config for the ScoutChain end-to-end suite.
 *
 * Runs against the local docker-compose stack started by scripts/dev-up.sh.
 * All tests require:
 *   E2E_BACKEND_URL    – backend base URL   (default http://localhost:3000)
 *   E2E_RPC_URL        – Soroban RPC        (default http://localhost:8000/soroban/rpc)
 *   NETWORK_PASSPHRASE – local network passphrase (default Standalone)
 *
 * When any of these is missing, or the backend is not reachable, the whole
 * suite self-skips (exit 0) so CI can include the job without gating merges.
 */
import type { Config } from "jest";

const config: Config = {
  preset: "ts-jest",
  testEnvironment: "node",
  roots: ["<rootDir>"],
  testMatch: ["**/*.e2e.test.ts"],
  maxWorkers: 1, // tests share one local chain — run sequentially
  testTimeout: 120_000,
  transform: {
    "^.+\\.tsx?$": ["ts-jest", { tsconfig: { strict: true, esModuleInterop: true, skipLibCheck: true } }],
  },
};

export default config;

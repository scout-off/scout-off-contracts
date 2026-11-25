import { Pool } from "pg";
import { createApp } from "./app";
import { parseMasterKey } from "./services/evidenceKeyCrypto";
import {
  createSorobanGrantChecker,
  grantCheckerConfigFromEnv,
} from "./services/evidenceGrantChecker";

const jwtSecret = process.env.JWT_SECRET;
if (!jwtSecret) {
  throw new Error("JWT_SECRET is required");
}

const database = new Pool({ connectionString: process.env.DATABASE_URL });
const app = createApp({
  database: {
    query: <Row extends Record<string, unknown> = Record<string, unknown>>(
      text: string,
      values?: unknown[]
    ) => database.query<Row>(text, values),
  },
  grantChecker: createSorobanGrantChecker(grantCheckerConfigFromEnv()),
  masterKey: parseMasterKey(process.env.EVIDENCE_MASTER_KEY),
  jwtSecret,
  isTokenRevoked: async (jti) => {
    const result = await database.query(
      "SELECT 1 FROM revoked_tokens WHERE jti = $1 LIMIT 1",
      [jti]
    );
    return result.rowCount !== 0;
  },
});

const port = Number(process.env.PORT ?? 3000);
const server = app.listen(port, () => {
  console.log(`ScoutChain Evidence API listening on http://localhost:${port}`);
});

function shutdown(): void {
  server.close(() => {
    void database.end();
  });
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);

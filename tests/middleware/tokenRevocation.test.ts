import { randomBytes } from "node:crypto";
import jwt from "jsonwebtoken";
import request from "supertest";
import { createApp } from "../../src/app";
import { issueSep10Token } from "../../src/services/sep10";
import type { EvidenceDatabase } from "../../src/routes/evidence";

const secret = "token-revocation-test-secret";
const revokedTokens = new Set<string>();

const database: EvidenceDatabase = {
  async query<Row extends Record<string, unknown> = Record<string, unknown>>(
    _text: string
  ) {
    return { rows: [] as Row[], rowCount: 1 };
  },
};

const app = createApp({
  database,
  grantChecker: { hasEvidenceAccess: async () => false },
  masterKey: randomBytes(32),
  jwtSecret: secret,
  isTokenRevoked: async (jti) => revokedTokens.has(jti),
});

beforeEach(() => revokedTokens.clear());

describe("token revocation", () => {
  it("accepts a signed SEP-10 JWT with a jti claim", async () => {
    const token = issueSep10Token(
      { sub: "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF", role: "validator" },
      secret
    );

    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer ${token}`)
      .send({ token })
      .expect(200)
      .expect(({ body }) => {
        expect(body.jti).toBeDefined();
        expect(body.role).toBe("validator");
      });
  });

  it("rejects a revoked token", async () => {
    const token = issueSep10Token(
      { sub: "wallet", role: "validator" },
      secret
    );
    const { jti } = jwt.verify(token, secret) as jwt.JwtPayload;
    revokedTokens.add(jti as string);

    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer ${token}`)
      .expect(401);
  });

  it("requires a jti claim", async () => {
    const token = jwt.sign(
      { sub: "wallet", role: "validator" },
      secret,
      { jwtid: randomBytes(8).toString("hex") }
    );
    const withoutJti = jwt.sign({ sub: "wallet", role: "validator" }, secret);

    expect(jwt.verify(token, secret)).toBeDefined();
    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer ${withoutJti}`)
      .expect(401);
  });
});

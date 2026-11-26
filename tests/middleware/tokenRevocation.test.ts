import { randomBytes } from "node:crypto";
import jwt from "jsonwebtoken";
import express from "express";
import { app } from "../../src/app";
import { issueSep10Token } from "../../src/services/sep10";
import { prisma } from "../../src/db";
import { requireAuth } from "../../src/middleware/auth";

const secret = "token-revocation-test-secret";
const revokedTokens = new Set<string>();

const database: EvidenceDatabase = {
  async query<Row extends Record<string, unknown> = Record<string, unknown>>(
    _text: string
  ) {
    return { rows: [] as Row[], rowCount: 1 };
  },
};

beforeEach(async () => {
  await prisma.revokedToken.deleteMany();
});

beforeEach(() => {
  process.env.JWT_SECRET = secret;
  jest.clearAllMocks();
  db.$transaction.mockImplementation((callback) => callback(db));
  db.revokedToken.findUnique.mockResolvedValue(null);
  db.revokedToken.upsert.mockResolvedValue({});
  db.session.findUnique.mockResolvedValue({ revokedAt: null, id: "session-1", account: "user1" });
  db.session.findMany.mockResolvedValue([]);
  db.session.create.mockResolvedValue({});
  db.session.updateMany.mockResolvedValue({ count: 1 });
  db.refreshToken.create.mockResolvedValue({});
  db.refreshToken.updateMany.mockResolvedValue({ count: 1 });
});

describe("requireAuth unknown-role policy", () => {
  const buildApp = (allowUnknown: boolean) => {
    const testApp = express();
    testApp.get(
      "/protected",
      requireAuth({ allowUnknown }),
      (req, res) => {
        res.json({ ok: true, role: req.user?.role });
      }
    );
    return testApp;
  };

  it("rejects unknown-role tokens by default with 403 not_registered", async () => {
    const token = issueSep10Token(
      { sub: "unregistered-wallet", role: "unknown" },
      TEST_SECRET
    );

    const response = await request(buildApp(false))
      .get("/protected")
      .set("Authorization", `Bearer ${token}`)
      .expect(403);

    expect(response.body).toEqual(
      expect.objectContaining({ error: "not_registered" })
    );
  });

  it("accepts unknown-role tokens when allowUnknown is true", async () => {
    const token = issueSep10Token(
      { sub: "unregistered-wallet", role: "unknown" },
      TEST_SECRET
    );

    const response = await request(buildApp(true))
      .get("/protected")
      .set("Authorization", `Bearer ${token}`)
      .expect(200);

    expect(response.body).toEqual({ ok: true, role: "unknown" });
  });

  it("still accepts registered roles on default routes", async () => {
    const token = issueSep10Token(
      { sub: "player1", role: "player" },
      TEST_SECRET
    );

    const response = await request(buildApp(false))
      .get("/protected")
      .set("Authorization", `Bearer ${token}`)
      .expect(200);

    expect(response.body).toEqual({ ok: true, role: "player" });
  });
});

describe("requireAuth unknown-role policy", () => {
  const buildApp = (allowUnknown: boolean) => {
    const testApp = express();
    testApp.get(
      "/protected",
      requireAuth({ allowUnknown }),
      (req, res) => {
        res.json({ ok: true, role: req.user?.role });
      }
    );
    return testApp;
  };

  it("rejects unknown-role tokens by default with 403 not_registered", async () => {
    const token = issueSep10Token(
      { sub: "unregistered-wallet", role: "unknown" },
      TEST_SECRET
    );

    const response = await request(buildApp(false))
      .get("/protected")
      .set("Authorization", `Bearer ${token}`)
      .expect(403);

    expect(response.body).toEqual(
      expect.objectContaining({ error: "not_registered" })
    );
  });

  it("accepts unknown-role tokens when allowUnknown is true", async () => {
    const token = issueSep10Token(
      { sub: "unregistered-wallet", role: "unknown" },
      TEST_SECRET
    );

    const response = await request(buildApp(true))
      .get("/protected")
      .set("Authorization", `Bearer ${token}`)
      .expect(200);

    expect(response.body).toEqual({ ok: true, role: "unknown" });
  });

  it("still accepts registered roles on default routes", async () => {
    const token = issueSep10Token(
      { sub: "player1", role: "player" },
      TEST_SECRET
    );

    const response = await request(buildApp(false))
      .get("/protected")
      .set("Authorization", `Bearer ${token}`)
      .expect(200);

    expect(response.body).toEqual({ ok: true, role: "player" });
  });
});

describe("Token revocation through real SEP-10 flow", () => {
  it("should include jti claim in JWT issued through sep10 signing path", async () => {
    const token = await issueSep10Token(
      { sub: "user123", role: "validator" },
      migrationConfig
    );

    const decoded = jwt.verify(token, config.jwtSecret) as { jti?: string };

    const response = await request(app)
      .post("/auth/refresh")
      .send({ refreshToken: "already-rotated-token" })
      .expect(401);

    expect(response.body.error).toBe("refresh_token_reused");
    expect(db.session.updateMany).toHaveBeenCalledWith(expect.objectContaining({
      where: { id: "session-1", revokedAt: null },
      data: expect.objectContaining({ revokedReason: "refresh_token_reused" }),
    }));
    expect(db.refreshToken.updateMany).toHaveBeenCalledWith(expect.objectContaining({
      where: { sessionId: "session-1", revokedAt: null },
    }));
  });

describe("token revocation", () => {
  it("accepts a signed SEP-10 JWT with a jti claim", async () => {
    const token = issueSep10Token(
      { sub: "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF", role: "validator" },
      secret
    );

    const responses = await Promise.all([
      request(app).post("/auth/refresh").send({ refreshToken: "same-token" }),
      request(app).post("/auth/refresh").send({ refreshToken: "same-token" }),
    ]);

    expect(responses.map(({ status }) => status).sort()).toEqual([200, 401]);
    expect(responses.find(({ status }) => status === 401)?.body.error).toBe("refresh_token_reused");
    expect(db.session.updateMany).toHaveBeenCalledWith(expect.objectContaining({
      data: expect.objectContaining({ revokedReason: "refresh_token_reused" }),
    }));
  });

  it("logout revokes the current session and access-token jti", async () => {
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

  it("should accept a lowercase bearer scheme", async () => {
    const token = issueSep10Token(
      { sub: "user123", role: "validator" },
      TEST_SECRET
    );

    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `bearer ${token}`)
      .expect(200);
  });

  it("should accept extra whitespace between scheme and token", async () => {
    const token = issueSep10Token(
      { sub: "user123", role: "validator" },
      TEST_SECRET
    );

    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer  ${token}`)
      .expect(200);
  });

  it("should reject headers with extra segments", async () => {
    const response = await request(app)
      .get("/api/tokens/me")
      .set("Authorization", "Bearer a b")
      .expect(401);

    expect(response.headers["www-authenticate"]).toBe(
      'Bearer error="invalid_request"'
    );
  });

  it("should reject non-JWT tokens", async () => {
    const response = await request(app)
      .get("/api/tokens/me")
      .set("Authorization", "Bearer not-a-jwt")
      .expect(401);

    expect(response.headers["www-authenticate"]).toBe(
      'Bearer error="invalid_request"'
    );
  });

  it("should accept a lowercase bearer scheme", async () => {
    const token = issueSep10Token(
      { sub: "user123", role: "validator" },
      TEST_SECRET
    );

    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `bearer ${token}`)
      .expect(200);
  });

  it("should accept extra whitespace between scheme and token", async () => {
    const token = issueSep10Token(
      { sub: "user123", role: "validator" },
      TEST_SECRET
    );

    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer  ${token}`)
      .expect(200);
  });

  it("should reject headers with extra segments", async () => {
    const response = await request(app)
      .get("/api/tokens/me")
      .set("Authorization", "Bearer a b")
      .expect(401);

    expect(response.headers["www-authenticate"]).toBe(
      'Bearer error="invalid_request"'
    );
  });

  it("should reject non-JWT tokens", async () => {
    const response = await request(app)
      .get("/api/tokens/me")
      .set("Authorization", "Bearer not-a-jwt")
      .expect(401);

    expect(response.headers["www-authenticate"]).toBe(
      'Bearer error="invalid_request"'
    );
  });

  it("should return 400 if token does not contain jti claim (manual test helper tokens)", async () => {
    // Use an admin token to pass the auth middleware, but send a manual
    // (jti-free) token as the payload to revoke.
    const adminToken = await issueSep10Token(
      { sub: "admin1", role: "admin" },
      migrationConfig
    );
    const manualToken = jwt.sign(
      { sub: "user123", role: "validator" },
      config.jwtSecret
    );
    const withoutJti = jwt.sign({ sub: "wallet", role: "validator" }, secret);

    expect(jwt.verify(token, secret)).toBeDefined();
    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer ${withoutJti}`)
      .expect(401);
  });
});

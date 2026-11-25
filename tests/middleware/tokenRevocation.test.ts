import request from "supertest";
import jwt from "jsonwebtoken";
import { app } from "../../src/app";
import { issueSep10Token } from "../../src/services/sep10";
import { prisma } from "../../src/db";

import { config } from "../../src/config";

// In test, setupEnv sets JWT_ALLOW_HS256_MIGRATION=true so config has no
// signingKeys but allowHs256Migration=true with a known jwtSecret.
const migrationConfig = {
  signingKeys: null,
  allowHs256Migration: true,
  jwtSecret: config.jwtSecret,
};

beforeEach(async () => {
  await prisma.revokedToken.deleteMany();
});

afterAll(async () => {
  await prisma.$disconnect();
});

describe("Token revocation through real SEP-10 flow", () => {
  it("should include jti claim in JWT issued through sep10 signing path", async () => {
    const token = await issueSep10Token(
      { sub: "user123", role: "validator" },
      migrationConfig
    );

    const decoded = jwt.verify(token, config.jwtSecret) as { jti?: string };

    expect(decoded.jti).toBeDefined();
    expect(typeof decoded.jti).toBe("string");
  });

  it("should allow revoking a token via the admin endpoint when jti is present", async () => {
    // Issue an admin-role token so requireRole(["admin"]) is satisfied.
    const token = await issueSep10Token(
      { sub: "admin1", role: "admin" },
      migrationConfig
    );

    await request(app)
      .post("/api/admin/tokens/revoke")
      .set("Authorization", `Bearer ${token}`)
      .send({ token })
      .expect(200)
      .expect((res) => {
        expect(res.body.revoked).toBe(true);
      });
  });

  it("should block the revoked token on subsequent requests", async () => {
    const token = await issueSep10Token(
      { sub: "admin1", role: "admin" },
      migrationConfig
    );

    await request(app)
      .post("/api/admin/tokens/revoke")
      .set("Authorization", `Bearer ${token}`)
      .send({ token });

    const response = await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer ${token}`)
      .expect(401);

    expect(response.body.error).toContain("revoked");
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

    await request(app)
      .post("/api/admin/tokens/revoke")
      .set("Authorization", `Bearer ${adminToken}`)
      .send({ token: manualToken })
      .expect(400)
      .expect((res) => {
        expect(res.body.error).toContain("jti claim");
      });
  });
});

import request from "supertest";
import jwt from "jsonwebtoken";
import { app } from "../../src/app";
import { issueSep10Token } from "../../src/services/sep10";
import { prisma } from "../../src/db";

const TEST_SECRET = "test-secret";

// Point the app at the test secret for every test.
process.env.JWT_SECRET = TEST_SECRET;

beforeEach(async () => {
  await prisma.revokedToken.deleteMany();
});

afterAll(async () => {
  await prisma.$disconnect();
});

describe("Token revocation through real SEP-10 flow", () => {
  it("should include jti claim in JWT issued through sep10 signing path", () => {
    const token = issueSep10Token(
      { sub: "user123", role: "validator" },
      TEST_SECRET
    );

    const decoded = jwt.verify(token, TEST_SECRET) as { jti?: string };

    expect(decoded.jti).toBeDefined();
    expect(typeof decoded.jti).toBe("string");
  });

  it("should allow revoking a token via the admin endpoint when jti is present", async () => {
    // Issue an admin-role token so requireRole(["admin"]) is satisfied.
    const token = issueSep10Token(
      { sub: "admin1", role: "admin" },
      TEST_SECRET
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
    const token = issueSep10Token(
      { sub: "admin1", role: "admin" },
      TEST_SECRET
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
    const adminToken = issueSep10Token(
      { sub: "admin1", role: "admin" },
      TEST_SECRET
    );
    const manualToken = jwt.sign(
      { sub: "user123", role: "validator" },
      TEST_SECRET
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

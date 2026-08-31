import request from "supertest";
import app from "../../src/app";
import { issueSep10Token } from "../../src/services/sep10";
import { prisma } from "../../src/db";

beforeEach(async () => {
  await prisma.revoked_tokens.deleteMany();
});

afterAll(async () => {
  await prisma.$disconnect();
});

describe("Token revocation through real SEP-10 flow", () => {
  it("should include jti claim in JWT issued through sep10 signing path", async () => {
    const token = issueSep10Token({ sub: "user123", role: "validator" }, "test-secret");

    const decoded: any = jwt.verify(token, "test-secret");

    expect(decoded.jti).toBeDefined();
    expect(typeof decoded.jti).toBe("string");
  });

  it("should allow revoking a token via the admin endpoint when jti is present", async () => {
    const token = issueSep10Token({ sub: "user123", role: "validator" }, "test-secret");

    await request(app)
      .post("/api/admin/tokens/revoke")
      .set("Authorization", `Bearer ${token}`)
      .expect(200)
      .expect((res: any) => {
        expect(res.body.revoked).toBe(true);
      });
  });

  it("should block the revoked token on subsequent requests", async () => {
    const token = issueSep10Token({ sub: "user123", role: "validator" }, "test-secret");

    await request(app)
      .post("/api/admin/tokens/revoke")
      .set("Authorization", `Bearer ${token}`);

    const response = await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer ${token}`)
      .expect(401);

    expect(response.body.error).toContain("revoked");
  });

  it("should return 400 if token does not contain jti claim (manual test helper tokens)", async () => {
    const manualToken = jwt.sign({ sub: "user123", role: "validator" }, "test-secret");

    await request(app)
      .post("/api/admin/tokens/revoke")
      .send({ token: manualToken })
      .expect(400)
      .expect((res: any) => {
        expect(res.body.error).toContain("jti claim");
      });
  });
});
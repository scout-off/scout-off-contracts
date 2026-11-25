import request from "supertest";
import { register } from "../src/metrics";
import jwt from "jsonwebtoken";

// Mock services before importing app
jest.mock("../src/services/sep10", () => ({
  buildChallengeTx: jest.fn(),
  verifyChallengeTx: jest.fn(),
  issueSep10Token: jest.fn(),
  issueSession: jest.fn(),
  refreshSession: jest.fn(),
  deriveRole: jest.fn(() => "player"),
  sep10Config: jest.fn(() => ({
    serverKeypair: { publicKey: "GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N" },
    homeDomain: "test.com",
    webAuthDomain: "auth.test.com",
    networkPassphrase: "test",
  })),
}));

// Mock prisma before importing app
jest.mock("../src/db", () => ({
  prisma: {
    revokedToken: {
      create: jest.fn(),
      findFirst: jest.fn(() => null),
    },
    admin: {
      findUnique: jest.fn(() => null),
    },
    validator: {
      findUnique: jest.fn(() => null),
    },
    scout: {
      findUnique: jest.fn(() => null),
    },
    player: {
      findUnique: jest.fn(() => null),
    },
  },
}));

// Import app after all mocks
import { app } from "../src/app";

describe("GET /metrics", () => {
  beforeEach(() => {
    delete process.env.METRICS_TOKEN;
    register.resetMetrics();
  });

  describe("without METRICS_TOKEN configured", () => {
    it("returns 200 OK with Prometheus format", async () => {
      const res = await request(app).get("/metrics");
      expect(res.status).toBe(200);
      expect(res.headers["content-type"]).toContain("text/plain");
      expect(res.text).toContain("# HELP");
      expect(res.text).toContain("# TYPE");
    });

    it("includes process metrics (nodejs_eventloop_lag_seconds)", async () => {
      const res = await request(app).get("/metrics");
      expect(res.text).toContain("nodejs_eventloop_lag_seconds");
    });

    it("includes http_request_duration_seconds histogram", async () => {
      const res = await request(app).get("/metrics");
      expect(res.text).toContain("http_request_duration_seconds");
    });
  });

  describe("with METRICS_TOKEN configured", () => {
    const testToken = "test-metrics-token";

    beforeEach(() => {
      process.env.METRICS_TOKEN = testToken;
    });

    afterEach(() => {
      delete process.env.METRICS_TOKEN;
    });

    it("returns 401 when no token provided", async () => {
      const res = await request(app).get("/metrics");
      expect(res.status).toBe(401);
      expect(res.body).toEqual({ error: "Unauthorized" });
    });

    it("returns 401 when invalid token provided", async () => {
      const res = await request(app).get("/metrics?token=wrong-token");
      expect(res.status).toBe(401);
    });

    it("returns 200 when token provided via query parameter", async () => {
      const res = await request(app).get(`/metrics?token=${testToken}`);
      expect(res.status).toBe(200);
    });

    it("returns 200 when token provided via Authorization header", async () => {
      const res = await request(app)
        .get("/metrics")
        .set("Authorization", `Bearer ${testToken}`);
      expect(res.status).toBe(200);
    });
  });

  describe("counter increments", () => {
    beforeEach(() => {
      delete process.env.METRICS_TOKEN;
      register.resetMetrics();
    });

    it("increments auth_challenges_issued_total after successful GET /auth", async () => {
      const initialMetrics = await request(app).get("/metrics");
      const initialChallengesMatch = initialMetrics.text.match(/auth_challenges_issued_total (\d+)/);
      const initialChallenges = initialChallengesMatch ? parseInt(initialChallengesMatch[1], 10) : 0;

      await request(app).get("/auth").query({ account: "GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N" });

      const finalMetrics = await request(app).get("/metrics");
      const finalChallengesMatch = finalMetrics.text.match(/auth_challenges_issued_total (\d+)/);
      const finalChallenges = finalChallengesMatch ? parseInt(finalChallengesMatch[1], 10) : 0;

      expect(finalChallenges).toBe(initialChallenges + 1);
    });

    it("increments auth_verifications_total with result=ok after successful POST /auth", async () => {
      const initialMetrics = await request(app).get("/metrics");
      const initialOkMatch = initialMetrics.text.match(/auth_verifications_total{result="ok"} (\d+)/);
      const initialOk = initialOkMatch ? parseInt(initialOkMatch[1], 10) : 0;

      // Mock services for successful auth
      (jest.mocked(require("../src/services/sep10").verifyChallengeTx)).mockResolvedValue("GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N");
      (jest.mocked(require("../src/services/sep10").issueSep10Token)).mockReturnValue("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ0ZXN0Iiwicm9sZSI6InBsYXllciIsImp0aSI6InRlc3QifQ.test");
      (jest.mocked(require("../src/services/sep10").issueSession)).mockReturnValue({ token: "token", refreshToken: "refresh", expiresIn: 900 });
      (jest.mocked(require("../src/services/sep10").deriveRole)).mockReturnValue("player");

      await request(app).post("/auth").send({
        transaction: "test-transaction",
        account: "GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N",
      });

      const finalMetrics = await request(app).get("/metrics");
      const finalOkMatch = finalMetrics.text.match(/auth_verifications_total{result="ok"} (\d+)/);
      const finalOk = finalOkMatch ? parseInt(finalOkMatch[1], 10) : 0;

      expect(finalOk).toBe(initialOk + 1);
    });

    it("increments auth_verifications_total with result=challenge_expired after expired challenge", async () => {
      const initialMetrics = await request(app).get("/metrics");
      const initialExpiredMatch = initialMetrics.text.match(/auth_verifications_total{result="challenge_expired"} (\d+)/);
      const initialExpired = initialExpiredMatch ? parseInt(initialExpiredMatch[1], 10) : 0;

      (jest.mocked(require("../src/services/sep10").verifyChallengeTx)).mockRejectedValue(new Error("The transaction has expired"));

      await request(app).post("/auth").send({
        transaction: "test-transaction",
        account: "GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N",
      });

      const finalMetrics = await request(app).get("/metrics");
      const finalExpiredMatch = finalMetrics.text.match(/auth_verifications_total{result="challenge_expired"} (\d+)/);
      const finalExpired = finalExpiredMatch ? parseInt(finalExpiredMatch[1], 10) : 0;

      expect(finalExpired).toBe(initialExpired + 1);
    });

    it("increments auth_refresh_total with result=ok after successful POST /auth/refresh", async () => {
      const initialMetrics = await request(app).get("/metrics");
      const initialOkMatch = initialMetrics.text.match(/auth_refresh_total{result="ok"} (\d+)/);
      const initialOk = initialOkMatch ? parseInt(initialOkMatch[1], 10) : 0;

      (jest.mocked(require("../src/services/sep10").refreshSession)).mockResolvedValue({
        token: "new-token",
        refreshToken: "new-refresh",
        expiresIn: 900,
      });

      await request(app).post("/auth/refresh").send({ refreshToken: "valid-refresh-token" });

      const finalMetrics = await request(app).get("/metrics");
      const finalOkMatch = finalMetrics.text.match(/auth_refresh_total{result="ok"} (\d+)/);
      const finalOk = finalOkMatch ? parseInt(finalOkMatch[1], 10) : 0;

      expect(finalOk).toBe(initialOk + 1);
    });

    it("increments auth_refresh_total with result=invalid after invalid refresh token", async () => {
      const initialMetrics = await request(app).get("/metrics");
      const initialInvalidMatch = initialMetrics.text.match(/auth_refresh_total{result="invalid"} (\d+)/);
      const initialInvalid = initialInvalidMatch ? parseInt(initialInvalidMatch[1], 10) : 0;

      (jest.mocked(require("../src/services/sep10").refreshSession)).mockRejectedValue(new Error("Invalid refresh token"));

      await request(app).post("/auth/refresh").send({ refreshToken: "invalid-refresh-token" });

      const finalMetrics = await request(app).get("/metrics");
      const finalInvalidMatch = finalMetrics.text.match(/auth_refresh_total{result="invalid"} (\d+)/);
      const finalInvalid = finalInvalidMatch ? parseInt(finalInvalidMatch[1], 10) : 0;

      expect(finalInvalid).toBe(initialInvalid + 1);
    });

    it("increments tokens_revoked_total after successful POST /api/admin/tokens/revoke", async () => {
      const initialMetrics = await request(app).get("/metrics");
      const initialRevokedMatch = initialMetrics.text.match(/tokens_revoked_total (\d+)/);
      const initialRevoked = initialRevokedMatch ? parseInt(initialRevokedMatch[1], 10) : 0;

      const adminToken = jwt.sign(
        { sub: "admin", role: "admin", jti: "admin-jti" },
        process.env.JWT_SECRET || "test",
        { algorithm: "HS256" }
      );

      await request(app)
        .post("/api/admin/tokens/revoke")
        .set("Authorization", `Bearer ${adminToken}`)
        .send({ token: "test-token" });

      const finalMetrics = await request(app).get("/metrics");
      const finalRevokedMatch = finalMetrics.text.match(/tokens_revoked_total (\d+)/);
      const finalRevoked = finalRevokedMatch ? parseInt(finalRevokedMatch[1], 10) : 0;

      expect(finalRevoked).toBe(initialRevoked + 1);
    });
  });
});

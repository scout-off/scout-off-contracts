import request from "supertest";
import jwt from "jsonwebtoken";

jest.mock("../../src/db", () => ({
  prisma: {
    session: {
      create: jest.fn(),
      findUnique: jest.fn(),
      findMany: jest.fn(),
      updateMany: jest.fn(),
    },
    refreshToken: {
      create: jest.fn(),
      findUnique: jest.fn(),
      updateMany: jest.fn(),
    },
    revokedToken: {
      findUnique: jest.fn(),
      upsert: jest.fn(),
    },
    $transaction: jest.fn(),
  },
}));

import { app } from "../../src/app";
import { issueSep10Token } from "../../src/services/sep10";
import { prisma } from "../../src/db";

const secret = "test-secret";
const db = prisma as unknown as {
  session: Record<string, jest.Mock>;
  refreshToken: Record<string, jest.Mock>;
  revokedToken: Record<string, jest.Mock>;
  $transaction: jest.Mock;
};

function accessToken(sub: string, role: string, sid?: string): string {
  return jwt.sign({ sub, role, ...(sid ? { sid } : {}) }, secret, { jwtid: "test-jti" });
}

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

describe("session-family auth", () => {
  it("issues sid-bearing access and refresh tokens", async () => {
    const response = await request(app)
      .post("/api/sep10/token")
      .send({ sub: "user1", role: "validator" })
      .expect(200);
    const payload = jwt.verify(response.body.accessToken, secret) as jwt.JwtPayload;

    expect(payload.sid).toBeTruthy();
    expect(response.body.refreshToken).toBeTruthy();
    expect(db.session.create).toHaveBeenCalled();
  });

  it("revokes the family and reports reuse of a rotated refresh token", async () => {
    db.refreshToken.findUnique.mockResolvedValue({
      id: "old-hash",
      sessionId: "session-1",
      role: "validator",
      revokedAt: new Date(),
      rotatedAt: new Date(),
      session: { account: "user1", revokedAt: null },
    });

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

  it("allows only one winner when two refresh requests race and revokes the family on reuse", async () => {
    const tokenRecord = {
      id: "old-hash",
      sessionId: "session-1",
      role: "validator",
      revokedAt: null as Date | null,
      rotatedAt: null as Date | null,
      session: { account: "user1", revokedAt: null },
    };
    db.refreshToken.findUnique.mockImplementation(({ include, select }) => {
      if (include) return Promise.resolve({ ...tokenRecord });
      if (select) return Promise.resolve({ rotatedAt: tokenRecord.rotatedAt, sessionId: "session-1" });
      return Promise.resolve({ ...tokenRecord });
    });
    db.session.findUnique.mockResolvedValue({ revokedAt: null });
    db.refreshToken.updateMany.mockImplementation(async () => {
      if (tokenRecord.rotatedAt) return { count: 0 };
      tokenRecord.rotatedAt = new Date();
      tokenRecord.revokedAt = tokenRecord.rotatedAt;
      return { count: 1 };
    });

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
      .post("/auth/logout")
      .set("Authorization", `Bearer ${accessToken("user1", "validator", "session-1")}`)
      .expect(200);

    expect(db.session.updateMany).toHaveBeenCalledWith(expect.objectContaining({
      where: { id: "session-1", revokedAt: null },
      data: expect.objectContaining({ revokedReason: "logout" }),
    }));
    expect(db.revokedToken.upsert).toHaveBeenCalledWith(expect.objectContaining({
      where: { jti: "test-jti" },
    }));
  });

  it("logout-all revokes every session for the authenticated account", async () => {
    db.session.findMany.mockResolvedValue([{ id: "session-1" }, { id: "session-2" }]);
    db.session.updateMany.mockResolvedValueOnce({ count: 2 });
    await request(app)
      .post("/auth/logout-all")
      .set("Authorization", `Bearer ${accessToken("user1", "validator", "session-1")}`)
      .expect(200)
      .expect(({ body }) => expect(body.sessionsRevoked).toBe(2));

    expect(db.session.updateMany).toHaveBeenCalledWith(expect.objectContaining({
      where: { account: "user1", revokedAt: null },
      data: expect.objectContaining({ revokedReason: "logout_all" }),
    }));
    expect(db.refreshToken.updateMany).toHaveBeenCalledWith(expect.objectContaining({
      where: { session: { is: { account: "user1" } }, revokedAt: null },
    }));
  });

  it("rejects access tokens whose session has been revoked", async () => {
    db.session.findUnique.mockResolvedValue({ revokedAt: new Date() });
    await request(app)
      .get("/api/tokens/me")
      .set("Authorization", `Bearer ${accessToken("user1", "validator", "session-1")}`)
      .expect(401)
      .expect(({ body }) => expect(body.error).toContain("session has been revoked"));
  });

  it("retains jti-based revocation support for pre-session tokens", async () => {
    const token = issueSep10Token({ sub: "user1", role: "validator" }, secret);
    const payload = jwt.verify(token, secret) as jwt.JwtPayload;

    await request(app)
      .post("/api/admin/tokens/revoke")
      .set("Authorization", `Bearer ${accessToken("admin1", "admin")}`)
      .send({ token })
      .expect(200);

    expect(db.revokedToken.upsert).toHaveBeenCalledWith(expect.objectContaining({
      where: { jti: payload.jti },
    }));
  });

  it("allows an admin to revoke every session for an account", async () => {
    db.session.findMany.mockResolvedValue([{ id: "session-a" }, { id: "session-b" }]);
    db.session.updateMany.mockResolvedValueOnce({ count: 2 });
    await request(app)
      .post("/api/admin/tokens/revoke")
      .set("Authorization", `Bearer ${accessToken("admin1", "admin")}`)
      .send({ account: "user1" })
      .expect(200)
      .expect(({ body }) => expect(body.sessionsRevoked).toBe(2));

    expect(db.session.updateMany).toHaveBeenCalledWith(expect.objectContaining({
      where: { account: "user1", revokedAt: null },
      data: expect.objectContaining({ revokedReason: "admin_revoke" }),
    }));
  });
});
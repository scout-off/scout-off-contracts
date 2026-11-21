import request from "supertest";
import jwt from "jsonwebtoken";

jest.mock("../src/db", () => ({ prisma: {} }));
jest.mock("../src/services/sep10", () => ({
  buildChallengeTx: jest.fn(),
  verifyChallengeTx: jest.fn(),
  deriveRole: jest.fn(),
  issueSep10Token: jest.fn(),
}));

import { app } from "../src/app";
import { buildChallengeTx, verifyChallengeTx } from "../src/services/sep10";
import { config } from "../src/config";
import { verifyToken } from "../src/middleware/auth";

const VALID_ACCOUNT = "GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N";

describe("GET /auth", () => {
  beforeEach(() => jest.spyOn(console, "error").mockImplementation(() => {}));
  afterEach(() => jest.restoreAllMocks());

  it("returns 400 when account is missing", async () => {
    const res = await request(app).get("/auth");
    expect(res.status).toBe(400);
  });

  it("returns 400 invalid account for a malformed account", async () => {
    const res = await request(app).get("/auth").query({ account: "not-a-key" });
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid account" });
    expect(buildChallengeTx).not.toHaveBeenCalled();
  });

  it("returns 400 invalid account when account is an array", async () => {
    const res = await request(app).get(`/auth?account=${VALID_ACCOUNT}&account=${VALID_ACCOUNT}`);
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid account" });
  });

  it("returns a generic 500 body without the exception message", async () => {
    (buildChallengeTx as jest.Mock).mockImplementation(() => {
      throw new Error("secret internal detail");
    });
    const res = await request(app).get("/auth").query({ account: VALID_ACCOUNT });
    expect(res.status).toBe(500);
    expect(res.body).toEqual({ error: "Internal server error" });
    expect(JSON.stringify(res.body)).not.toContain("secret internal detail");
  });
});

describe("POST /auth", () => {
  beforeEach(() => {
    jest.spyOn(console, "error").mockImplementation(() => {});
    jest.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => jest.restoreAllMocks());

  it("maps verification failures to stable codes", async () => {
    (verifyChallengeTx as jest.Mock).mockRejectedValue(
      new Error("The transaction has expired")
    );
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "tx", account: VALID_ACCOUNT });
    expect(res.status).toBe(401);
    expect(res.body).toEqual({ error: "challenge_expired" });
  });

  it("does not echo unexpected error messages", async () => {
    (verifyChallengeTx as jest.Mock).mockRejectedValue(new Error("db connection refused"));
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "tx", account: VALID_ACCOUNT });
    expect(res.status).toBe(500);
    expect(JSON.stringify(res.body)).not.toContain("db connection refused");
  });
});

describe("JWT iss/aud enforcement", () => {
  const sign = (payload: object, options: jwt.SignOptions = {}) =>
    jwt.sign(payload, config.jwtSecret, {
      algorithm: "HS256",
      issuer: config.jwtIssuer,
      audience: config.jwtAudience,
      ...options,
    });

  it("accepts a token with the correct iss and aud", () => {
    const token = sign({ sub: VALID_ACCOUNT, role: "user" });
    const decoded = verifyToken(token);
    expect(decoded.sub).toBe(VALID_ACCOUNT);
    expect(decoded.iss).toBe(config.jwtIssuer);
    expect(decoded.aud).toBe(config.jwtAudience);
  });

  it("rejects a token with the wrong audience", () => {
    const token = sign({ sub: VALID_ACCOUNT, role: "user" }, { audience: "other-service" });
    expect(() => verifyToken(token)).toThrow();
  });

  it("rejects a token with the wrong issuer", () => {
    const token = sign({ sub: VALID_ACCOUNT, role: "user" }, { issuer: "https://evil.example" });
    expect(() => verifyToken(token)).toThrow();
  });

  it("rejects a token with no iss or aud", () => {
    const token = jwt.sign({ sub: VALID_ACCOUNT, role: "user" }, config.jwtSecret, {
      algorithm: "HS256",
    });
    expect(() => verifyToken(token)).toThrow();
  });

  it("returns 401 on a protected route for a token with the wrong audience", async () => {
    const token = sign({ sub: VALID_ACCOUNT, role: "admin" }, { audience: "other-service" });
    const res = await request(app)
      .post("/admin/revoke")
      .set("Authorization", `Bearer ${token}`)
      .send({ jti: "some-jti" });
    expect(res.status).toBe(401);
  });

  it("returns 401 on a protected route for a token with the wrong issuer", async () => {
    const token = sign({ sub: VALID_ACCOUNT, role: "admin" }, { issuer: "https://evil.example" });
    const res = await request(app)
      .post("/admin/revoke")
      .set("Authorization", `Bearer ${token}`)
      .send({ jti: "some-jti" });
    expect(res.status).toBe(401);
  });
});

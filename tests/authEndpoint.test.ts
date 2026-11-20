import request from "supertest";

jest.mock("../src/db", () => ({ prisma: {} }));
jest.mock("../src/services/sep10", () => ({
  buildChallengeTx: jest.fn(),
  verifyChallengeTx: jest.fn(),
  deriveRole: jest.fn(),
  issueSep10Token: jest.fn(),
}));

import { app } from "../src/app";
import {
  buildChallengeTx,
  verifyChallengeTx,
  deriveRole,
  issueSep10Token,
} from "../src/services/sep10";

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

describe("unknown-role policy", () => {
  beforeEach(() => {
    jest.spyOn(console, "error").mockImplementation(() => {});
    jest.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => jest.restoreAllMocks());

  it("issues an onboarding-scoped token with the short refresh TTL for an unknown wallet", async () => {
    (verifyChallengeTx as jest.Mock).mockResolvedValue({ account: VALID_ACCOUNT });
    (deriveRole as jest.Mock).mockResolvedValue("unknown");
    (issueSep10Token as jest.Mock).mockResolvedValue({
      accessToken: "access-unknown",
      refreshToken: "refresh-unknown",
      role: "unknown",
      scope: "onboarding",
      refreshExpiresIn: 3600,
    });

    const res = await request(app)
      .post("/auth")
      .send({ transaction: "tx", account: VALID_ACCOUNT });

    expect(res.status).toBe(200);
    expect(res.body.role).toBe("unknown");
    expect(res.body.scope).toBe("onboarding");
    expect(res.body.refreshExpiresIn).toBe(3600);
    expect(issueSep10Token).toHaveBeenCalledWith(
      expect.objectContaining({ role: "unknown" })
    );
  });

  it("rejects an unknown-role token on a protected route by default", async () => {
    const res = await request(app)
      .get("/api/validators")
      .set("Authorization", "Bearer unknown-role-token");
    expect(res.status).toBe(403);
    expect(res.body).toEqual({ error: "not_registered" });
  });

  it("accepts an unknown-role token on the opt-in /api/tokens/me route", async () => {
    const res = await request(app)
      .get("/api/tokens/me")
      .set("Authorization", "Bearer unknown-role-token");
    expect(res.status).toBe(200);
    expect(res.body.role).toBe("unknown");
    expect(res.body.scope).toBe("onboarding");
  });

  it("returns the real role after the wallet registers and refreshes", async () => {
    (deriveRole as jest.Mock).mockResolvedValue("player");
    const res = await request(app)
      .post("/api/tokens/refresh")
      .send({ refreshToken: "refresh-unknown" });
    expect(res.status).toBe(200);
    expect(res.body.role).toBe("player");
    expect(res.body.scope).not.toBe("onboarding");
  });
});

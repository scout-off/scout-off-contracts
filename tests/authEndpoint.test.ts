import request from "supertest";

jest.mock("../src/db", () => ({ prisma: {} }));
jest.mock("../src/services/sep10", () => ({
  buildChallengeTx: jest.fn(),
  verifyChallengeTx: jest.fn(),
  deriveRole: jest.fn(),
  issueSep10Token: jest.fn(),
}));

import { app } from "../src/app";
import { buildChallengeTx, verifyChallengeTx } from "../src/services/sep10";

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

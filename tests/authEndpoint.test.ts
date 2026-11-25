import request from "supertest";
import { logOutput, mockLogger } from "./setupEnv";

jest.mock("../src/db", () => ({ prisma: {} }));
jest.mock("../src/services/sep10", () => ({
  buildChallengeTx: jest.fn(),
  verifyChallengeTx: jest.fn(),
  deriveRole: jest.fn(() => "player"),
  issueSep10Token: jest.fn(() => "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ0ZXN0Iiwicm9sZSI6InBsYXllciIsImp0aSI6InRlc3QifQ.test"),
}));

// Mock the logger module BEFORE importing app.ts
jest.mock("../src/logger", () => ({
  logger: mockLogger,
  generateRequestId: () => "test-request-id",
  attachRequestIdToResponse: jest.fn((req, res) => {
    const requestId = (req as Request & { requestId?: string }).requestId;
    if (requestId) {
      res.setHeader("x-request-id", requestId);
    }
  }),
}));

import { app } from "../src/app";
import { buildChallengeTx, verifyChallengeTx } from "../src/services/sep10";

const VALID_ACCOUNT = "GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N";

describe("GET /auth", () => {
  beforeEach(() => {
    logOutput.length = 0;
  });

  it("returns 400 when account is missing", async () => {
    const res = await request(app).get("/auth");
    expect(res.status).toBe(400);
    // Check warning log was generated
    const warningLog = logOutput.find((l) => l.level === "warn");
    expect(warningLog).toBeDefined();
    expect(warningLog?.args[1]).toContain("missing account parameter");
  });

  it("returns 400 invalid account for a malformed account", async () => {
    const res = await request(app).get("/auth").query({ account: "not-a-key" });
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid account" });
    expect(buildChallengeTx).not.toHaveBeenCalled();
    // Check warning log was generated
    const warningLog = logOutput.find((l) => l.level === "warn");
    expect(warningLog).toBeDefined();
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
    // Check error log was generated but without raw error object
    const errorLog = logOutput.find((l) => l.level === "error");
    expect(errorLog).toBeDefined();
    // The error message should be stringified, not the raw object
    expect(JSON.stringify(errorLog?.args)).not.toContain("secret internal detail");
  });
});

describe("POST /auth", () => {
  beforeEach(() => {
    logOutput.length = 0;
  });

  it("rejects a non-string transaction with 'invalid transaction'", async () => {
    for (const transaction of [{ foo: "bar" }, 123, ["tx"]]) {
      const res = await request(app)
        .post("/auth")
        .send({ transaction, account: VALID_ACCOUNT });
      expect(res.status).toBe(400);
      expect(res.body).toEqual({ error: "invalid transaction" });
    }
  });

  it("rejects an invalid account with 'invalid account'", async () => {
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "tx", account: "not-a-key" });
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid account" });
  });

  it("maps verification failures to stable codes", async () => {
    (verifyChallengeTx as jest.Mock).mockRejectedValue(
      new Error("The transaction has expired")
    );
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "tx", account: VALID_ACCOUNT });
    expect(res.status).toBe(401);
    expect(res.body).toEqual({ error: "challenge_expired" });
    // Check warning log was generated
    const warningLog = logOutput.find((l) => l.level === "warn");
    expect(warningLog).toBeDefined();
    expect(warningLog?.args[1]).toContain("verification failed");
  });

  it("does not echo unexpected error messages", async () => {
    (verifyChallengeTx as jest.Mock).mockRejectedValue(new Error("db connection refused"));
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "tx", account: VALID_ACCOUNT });
    expect(res.status).toBe(500);
    expect(JSON.stringify(res.body)).not.toContain("db connection refused");
    // Check error log was generated with stringified error
    const errorLog = logOutput.find((l) => l.level === "error");
    expect(errorLog).toBeDefined();
    expect(JSON.stringify(errorLog?.args)).not.toContain("db connection refused");
  });
});

describe("x-request-id header", () => {
  it("returns x-request-id in response header", async () => {
    const res = await request(app).get("/auth").query({ account: VALID_ACCOUNT });
    expect(res.headers["x-request-id"]).toBeDefined();
    expect(typeof res.headers["x-request-id"]).toBe("string");
  });
});

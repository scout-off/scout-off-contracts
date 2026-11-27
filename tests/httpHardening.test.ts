import request from "supertest";

jest.mock("../src/db", () => ({ prisma: {} }));
jest.mock("../src/services/sep10", () => ({
  buildChallengeTx: jest.fn(),
  verifyChallengeTx: jest.fn(),
  issueSession: jest.fn(),
  refreshSession: jest.fn(),
}));

import { app } from "../src/app";

describe("HTTP hardening", () => {
  beforeAll(() => {
    process.env.CORS_ORIGINS = "https://app.example.com, https://b.example.com";
  });

  it("answers preflight for an allowed origin", async () => {
    const res = await request(app)
      .options("/auth")
      .set("Origin", "https://app.example.com")
      .set("Access-Control-Request-Method", "POST");
    expect(res.status).toBe(204);
    expect(res.headers["access-control-allow-origin"]).toBe("https://app.example.com");
    expect(res.headers["access-control-allow-credentials"]).toBeUndefined();
  });

  it("rejects preflight for a disallowed origin", async () => {
    const res = await request(app)
      .options("/auth")
      .set("Origin", "https://evil.example.com");
    expect(res.status).toBe(403);
    expect(res.headers["access-control-allow-origin"]).toBeUndefined();
  });

  it("returns JSON 400 for malformed JSON", async () => {
    const res = await request(app)
      .post("/auth")
      .set("Content-Type", "application/json")
      .send("{not json");
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid_json" });
  });

  it("returns JSON 413 for oversized bodies", async () => {
    const res = await request(app)
      .post("/auth")
      .set("Content-Type", "application/json")
      .send(JSON.stringify({ transaction: "x".repeat(20 * 1024) }));
    expect(res.status).toBe(413);
    expect(res.body).toEqual({ error: "payload_too_large" });
  });

  it("omits x-powered-by and sets security headers", async () => {
    const res = await request(app).get("/auth");
    expect(res.headers["x-powered-by"]).toBeUndefined();
    expect(res.headers["x-content-type-options"]).toBe("nosniff");
  });
});

import express from "express";
import request from "supertest";
import { requireRole } from "../../src/middleware/auth";

describe("requireRole middleware", () => {
  const makeRes = () => {
    const res: any = {};
    res.status = jest.fn().mockReturnValue(res);
    res.json = jest.fn().mockReturnValue(res);
    return res;
  };

  it("returns 401 when req.user is missing", () => {
    const req: any = {};
    const res = makeRes();
    const next = jest.fn();

    requireRole(["admin"])(req, res, next);

    expect(res.status).toHaveBeenCalledWith(401);
    expect(res.json).toHaveBeenCalledWith({ error: "Unauthorized - no user role" });
    expect(next).not.toHaveBeenCalled();
  });

  it("returns 401 when req.user has no role", () => {
    const req: any = { user: {} };
    const res = makeRes();
    const next = jest.fn();

    requireRole(["admin"])(req, res, next);

    expect(res.status).toHaveBeenCalledWith(401);
    expect(res.json).toHaveBeenCalledWith({ error: "Unauthorized - no user role" });
    expect(next).not.toHaveBeenCalled();
  });

  it("returns 403 when role is not in the allowed list", () => {
    const req: any = { user: { role: "user" } };
    const res = makeRes();
    const next = jest.fn();

    requireRole(["admin"])(req, res, next);

    expect(res.status).toHaveBeenCalledWith(403);
    expect(res.json).toHaveBeenCalledWith({ error: "Forbidden - insufficient role" });
    expect(next).not.toHaveBeenCalled();
  });

  it("calls next when role is in the allowed list", () => {
    const req: any = { user: { role: "admin" } };
    const res = makeRes();
    const next = jest.fn();

    requireRole(["admin"])(req, res, next);

    expect(next).toHaveBeenCalledTimes(1);
    expect(res.status).not.toHaveBeenCalled();
    expect(res.json).not.toHaveBeenCalled();
  });

  it("returns 401 when mounted without requireAuth first", async () => {
    const app = express();
    app.get("/admin", requireRole(["admin"]), (_req, res) => {
      res.json({ ok: true });
    });

    const response = await request(app).get("/admin");

    expect(response.status).toBe(401);
    expect(response.body).toEqual({ error: "Unauthorized - no user role" });
  });
});
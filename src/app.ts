import express, { ErrorRequestHandler, Request, Response } from "express";
import jwt from "jsonwebtoken";
import { prisma } from "./db";
import { clearSessionRevocationCache, getConfiguredJwtSecret, requireAuth, requireRole } from "./middleware/auth";
import { createSession, refreshSession, revokeAccountSessions, revokeSession, revokeSessionById, SessionError } from "./services/sessions";

const app = express();
app.use(express.json());

app.get("/", (_req: Request, res: Response) => {
  res.type("html").send(`<!doctype html>
<html lang="en">
  <head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>ScoutChain Auth API</title></head>
  <body>
    <main>
      <h1>ScoutChain Auth API</h1>
      <p>Session management service is running.</p>
      <p>Available endpoints: <code>POST /api/sep10/token</code>, <code>POST /auth/refresh</code>, <code>POST /auth/logout</code>, <code>POST /auth/logout-all</code>.</p>
    </main>
  </body>
</html>`);
});

app.get("/health", (_req: Request, res: Response) => {
  res.json({ status: "ok" });
});

app.post("/api/sep10/token", async (req: Request, res: Response, next) => {
  const { sub, role } = req.body ?? {};
  if (typeof sub !== "string" || !sub || typeof role !== "string" || !role) {
    res.status(400).json({ error: "sub and role are required" });
    return;
  }

  try {
    const tokens = await createSession(sub, role, getConfiguredJwtSecret());
    res.json({ token: tokens.accessToken, accessToken: tokens.accessToken, refreshToken: tokens.refreshToken });
  } catch (error) {
    next(error);
  }
});

app.post("/auth/refresh", async (req: Request, res: Response, next) => {
  const { refreshToken } = req.body ?? {};
  if (typeof refreshToken !== "string" || !refreshToken) {
    res.status(400).json({ error: "refreshToken is required" });
    return;
  }

  try {
    const tokens = await refreshSession(refreshToken, getConfiguredJwtSecret());
    res.json(tokens);
  } catch (error) {
    if (error instanceof SessionError) {
      res.status(401).json({ error: error.code });
      return;
    }
    next(error);
  }
});

app.post("/auth/logout", requireAuth, async (req: Request, res: Response, next) => {
  try {
    const identity = req.user!;
    if (identity.sid) {
      await revokeSession(identity.sid, identity.sub, "logout");
      clearSessionRevocationCache([identity.sid]);
    }
    await prisma.revokedToken.upsert({
      where: { jti: identity.jti },
      create: { jti: identity.jti },
      update: {},
    });
    res.json({ revoked: true });
  } catch (error) {
    next(error);
  }
});

app.post("/auth/logout-all", requireAuth, async (req: Request, res: Response, next) => {
  try {
    const identity = req.user!;
    const sessionsRevoked = await revokeAccountSessions(identity.sub, "logout_all");
    await prisma.revokedToken.upsert({
      where: { jti: identity.jti },
      create: { jti: identity.jti },
      update: {},
    });
    res.json({ revoked: true, sessionsRevoked });
  } catch (error) {
    next(error);
  }
});

app.post(
  "/api/admin/tokens/revoke",
  requireAuth,
  requireRole(["admin"]),
  async (req: Request, res: Response, next) => {
    const { sid, account, token } = req.body ?? {};
    if ([sid, account, token].filter((value) => value !== undefined).length !== 1) {
      res.status(400).json({ error: "Provide exactly one of sid, account, or token" });
      return;
    }

    try {
      if (typeof sid === "string" && sid) {
        const revoked = await revokeSessionById(sid, "admin_revoke");
        res.json({ revoked, sessionsRevoked: revoked ? 1 : 0 });
        return;
      }
      if (typeof account === "string" && account) {
        const sessionsRevoked = await revokeAccountSessions(account, "admin_revoke");
        res.json({ revoked: sessionsRevoked > 0, sessionsRevoked });
        return;
      }
      if (typeof token === "string" && token) {
        const secret = getConfiguredJwtSecret();
        let payload: jwt.JwtPayload;
        try {
          const verified = jwt.verify(token, secret);
          if (typeof verified === "string") {
            res.status(400).json({ error: "Invalid token" });
            return;
          }
          payload = verified;
        } catch {
          res.status(400).json({ error: "Invalid token" });
          return;
        }
        if (typeof payload.sid === "string") {
          const revoked = await revokeSessionById(payload.sid, "admin_revoke");
          res.json({ revoked, sessionsRevoked: revoked ? 1 : 0 });
          return;
        }
        if (typeof payload.jti !== "string") {
          res.status(400).json({ error: "Token does not contain a jti claim" });
          return;
        }
        await prisma.revokedToken.upsert({
          where: { jti: payload.jti },
          create: { jti: payload.jti },
          update: {},
        });
        res.json({ revoked: true, sessionsRevoked: 0 });
        return;
      }
      res.status(400).json({ error: "Provide a non-empty sid, account, or token" });
    } catch (error) {
      next(error);
    }
  },
);

app.get("/api/tokens/me", requireAuth, (req: Request, res: Response) => {
  const { sub, role, jti, sid } = req.user!;
  res.json({ sub, role, jti, ...(sid ? { sid } : {}) });
});

const errorHandler: ErrorRequestHandler = (error, _req, res, _next) => {
  console.error("Auth API request failed", error);
  res.status(500).json({ error: "Internal server error" });
};
app.use(errorHandler);

export { app };

"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.app = void 0;
const express_1 = __importDefault(require("express"));
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const db_1 = require("./db");
const auth_1 = require("./middleware/auth");
const sessions_1 = require("./services/sessions");
const app = (0, express_1.default)();
exports.app = app;
app.use(express_1.default.json());
app.get("/", (_req, res) => {
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
app.get("/health", (_req, res) => {
    res.json({ status: "ok" });
});
app.post("/api/sep10/token", async (req, res, next) => {
    const { sub, role } = req.body ?? {};
    if (typeof sub !== "string" || !sub || typeof role !== "string" || !role) {
        res.status(400).json({ error: "sub and role are required" });
        return;
    }
    try {
        const tokens = await (0, sessions_1.createSession)(sub, role, (0, auth_1.getConfiguredJwtSecret)());
        res.json({ token: tokens.accessToken, accessToken: tokens.accessToken, refreshToken: tokens.refreshToken });
    }
    catch (error) {
        next(error);
    }
});
app.post("/auth/refresh", async (req, res, next) => {
    const { refreshToken } = req.body ?? {};
    if (typeof refreshToken !== "string" || !refreshToken) {
        res.status(400).json({ error: "refreshToken is required" });
        return;
    }
    try {
        const tokens = await (0, sessions_1.refreshSession)(refreshToken, (0, auth_1.getConfiguredJwtSecret)());
        res.json(tokens);
    }
    catch (error) {
        if (error instanceof sessions_1.SessionError) {
            res.status(401).json({ error: error.code });
            return;
        }
        next(error);
    }
});
app.post("/auth/logout", auth_1.requireAuth, async (req, res, next) => {
    try {
        const identity = req.user;
        if (identity.sid) {
            await (0, sessions_1.revokeSession)(identity.sid, identity.sub, "logout");
            (0, auth_1.clearSessionRevocationCache)([identity.sid]);
        }
        await db_1.prisma.revokedToken.upsert({
            where: { jti: identity.jti },
            create: { jti: identity.jti },
            update: {},
        });
        res.json({ revoked: true });
    }
    catch (error) {
        next(error);
    }
});
app.post("/auth/logout-all", auth_1.requireAuth, async (req, res, next) => {
    try {
        const identity = req.user;
        const sessionsRevoked = await (0, sessions_1.revokeAccountSessions)(identity.sub, "logout_all");
        await db_1.prisma.revokedToken.upsert({
            where: { jti: identity.jti },
            create: { jti: identity.jti },
            update: {},
        });
        res.json({ revoked: true, sessionsRevoked });
    }
    catch (error) {
        next(error);
    }
});
app.post("/api/admin/tokens/revoke", auth_1.requireAuth, (0, auth_1.requireRole)(["admin"]), async (req, res, next) => {
    const { sid, account, token } = req.body ?? {};
    if ([sid, account, token].filter((value) => value !== undefined).length !== 1) {
        res.status(400).json({ error: "Provide exactly one of sid, account, or token" });
        return;
    }
    try {
        if (typeof sid === "string" && sid) {
            const revoked = await (0, sessions_1.revokeSessionById)(sid, "admin_revoke");
            res.json({ revoked, sessionsRevoked: revoked ? 1 : 0 });
            return;
        }
        if (typeof account === "string" && account) {
            const sessionsRevoked = await (0, sessions_1.revokeAccountSessions)(account, "admin_revoke");
            res.json({ revoked: sessionsRevoked > 0, sessionsRevoked });
            return;
        }
        if (typeof token === "string" && token) {
            const secret = (0, auth_1.getConfiguredJwtSecret)();
            let payload;
            try {
                const verified = jsonwebtoken_1.default.verify(token, secret);
                if (typeof verified === "string") {
                    res.status(400).json({ error: "Invalid token" });
                    return;
                }
                payload = verified;
            }
            catch {
                res.status(400).json({ error: "Invalid token" });
                return;
            }
            if (typeof payload.sid === "string") {
                const revoked = await (0, sessions_1.revokeSessionById)(payload.sid, "admin_revoke");
                res.json({ revoked, sessionsRevoked: revoked ? 1 : 0 });
                return;
            }
            if (typeof payload.jti !== "string") {
                res.status(400).json({ error: "Token does not contain a jti claim" });
                return;
            }
            await db_1.prisma.revokedToken.upsert({
                where: { jti: payload.jti },
                create: { jti: payload.jti },
                update: {},
            });
            res.json({ revoked: true, sessionsRevoked: 0 });
            return;
        }
        res.status(400).json({ error: "Provide a non-empty sid, account, or token" });
    }
    catch (error) {
        next(error);
    }
});
app.get("/api/tokens/me", auth_1.requireAuth, (req, res) => {
    const { sub, role, jti, sid } = req.user;
    res.json({ sub, role, jti, ...(sid ? { sid } : {}) });
});
const errorHandler = (error, _req, res, _next) => {
    console.error("Auth API request failed", error);
    res.status(500).json({ error: "Internal server error" });
};
app.use(errorHandler);

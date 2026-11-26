"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.requireAuth = void 0;
exports.clearSessionRevocationCache = clearSessionRevocationCache;
exports.isTokenRevoked = isTokenRevoked;
exports.isSessionRevoked = isSessionRevoked;
exports.requireRole = requireRole;
exports.getConfiguredJwtSecret = getConfiguredJwtSecret;
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const db_1 = require("../db");
const SESSION_CACHE_TTL_MS = 2000;
const sessionRevocationCache = new Map();
function clearSessionRevocationCache(sessionIds) {
    for (const sessionId of sessionIds) {
        sessionRevocationCache.delete(sessionId);
    }
}
async function isTokenRevoked(jti) {
    return Boolean(await db_1.prisma.revokedToken.findUnique({ where: { jti } }));
}
async function isSessionRevoked(sid) {
    const cached = sessionRevocationCache.get(sid);
    if (cached && cached.expiresAt > Date.now()) {
        return cached.revoked;
    }
    const session = await db_1.prisma.session.findUnique({
        where: { id: sid },
        select: { revokedAt: true },
    });
    const revoked = !session || session.revokedAt !== null;
    sessionRevocationCache.set(sid, {
        revoked,
        expiresAt: Date.now() + SESSION_CACHE_TTL_MS,
    });
    return revoked;
}
function getJwtSecret() {
    const secret = process.env.JWT_SECRET;
    if (!secret) {
        throw new Error("JWT_SECRET is not configured");
    }
    return secret;
}
const requireAuth = async (req, res, next) => {
    const authHeader = req.headers.authorization;
    if (!authHeader?.startsWith("Bearer ")) {
        res.status(401).json({ error: "Unauthorized - no token provided" });
        return;
    }
    let secret;
    try {
        secret = getJwtSecret();
    }
    catch (error) {
        next(error);
        return;
    }
    let payload;
    try {
        const verified = jsonwebtoken_1.default.verify(authHeader.slice(7), secret);
        if (typeof verified === "string") {
            res.status(401).json({ error: "Unauthorized - invalid token" });
            return;
        }
        payload = verified;
    }
    catch {
        res.status(401).json({ error: "Unauthorized - invalid token" });
        return;
    }
    if (typeof payload.sub !== "string" ||
        typeof payload.role !== "string" ||
        typeof payload.jti !== "string") {
        res.status(401).json({ error: "Unauthorized - token is missing required claims" });
        return;
    }
    try {
        if (await isTokenRevoked(payload.jti)) {
            res.status(401).json({ error: "Unauthorized - token has been revoked" });
            return;
        }
        if (typeof payload.sid === "string" && await isSessionRevoked(payload.sid)) {
            res.status(401).json({ error: "Unauthorized - session has been revoked" });
            return;
        }
        req.user = {
            sub: payload.sub,
            role: payload.role,
            jti: payload.jti,
            ...(typeof payload.sid === "string" ? { sid: payload.sid } : {}),
        };
        next();
    }
    catch (error) {
        next(error);
    }
};
exports.requireAuth = requireAuth;
function requireRole(allowedRoles) {
    return (req, res, next) => {
        if (!req.user?.role) {
            res.status(401).json({ error: "Unauthorized - no user role" });
            return;
        }
        if (!allowedRoles.includes(req.user.role)) {
            res.status(403).json({ error: "Forbidden - insufficient role" });
            return;
        }
        next();
    };
}
function getConfiguredJwtSecret() {
    return getJwtSecret();
}

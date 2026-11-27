"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.SessionError = void 0;
exports.issueAccessToken = issueAccessToken;
exports.createSession = createSession;
exports.revokeSession = revokeSession;
exports.revokeAccountSessions = revokeAccountSessions;
exports.revokeSessionById = revokeSessionById;
exports.refreshSession = refreshSession;
const node_crypto_1 = require("node:crypto");
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const db_1 = require("../db");
const auth_1 = require("../middleware/auth");
const REFRESH_TOKEN_BYTES = 48;
class SessionError extends Error {
    constructor(code) {
        super(code);
        this.code = code;
    }
}
exports.SessionError = SessionError;
function hashRefreshToken(token) {
    return (0, node_crypto_1.createHash)("sha256").update(token).digest("hex");
}
function newRefreshToken() {
    return (0, node_crypto_1.randomBytes)(REFRESH_TOKEN_BYTES).toString("base64url");
}
function issueAccessToken(identity, secret) {
    return jsonwebtoken_1.default.sign(identity, secret, {
        expiresIn: "15m",
        jwtid: (0, node_crypto_1.randomUUID)(),
    });
}
async function createSession(account, role, secret) {
    const sessionId = (0, node_crypto_1.randomUUID)();
    const refreshToken = newRefreshToken();
    const now = new Date();
    await db_1.prisma.$transaction(async (tx) => {
        await tx.session.create({
            data: {
                id: sessionId,
                account,
                refreshTokens: {
                    create: {
                        id: hashRefreshToken(refreshToken),
                        role,
                        createdAt: now,
                    },
                },
            },
        });
    });
    return {
        accessToken: issueAccessToken({ sub: account, role, sid: sessionId }, secret),
        refreshToken,
    };
}
async function revokeFamily(sessionId, reason) {
    const now = new Date();
    await db_1.prisma.$transaction(async (tx) => {
        await tx.session.updateMany({
            where: { id: sessionId, revokedAt: null },
            data: { revokedAt: now, revokedReason: reason },
        });
        await tx.refreshToken.updateMany({
            where: { sessionId, revokedAt: null },
            data: { revokedAt: now },
        });
    });
    (0, auth_1.clearSessionRevocationCache)([sessionId]);
}
async function revokeSession(sessionId, account, reason) {
    const session = await db_1.prisma.session.findUnique({
        where: { id: sessionId },
        select: { id: true, account: true },
    });
    if (!session || session.account !== account) {
        return false;
    }
    await revokeFamily(sessionId, reason);
    return true;
}
async function revokeAccountSessions(account, reason) {
    const now = new Date();
    const sessions = await db_1.prisma.session.findMany({
        where: { account, revokedAt: null },
        select: { id: true },
    });
    if (sessions.length === 0) {
        return 0;
    }
    const sessionIds = sessions.map(({ id }) => id);
    const result = await db_1.prisma.$transaction(async (tx) => {
        const revoked = await tx.session.updateMany({
            where: { account, revokedAt: null },
            data: { revokedAt: now, revokedReason: reason },
        });
        await tx.refreshToken.updateMany({
            where: { session: { is: { account } }, revokedAt: null },
            data: { revokedAt: now },
        });
        return revoked.count;
    });
    (0, auth_1.clearSessionRevocationCache)(sessionIds);
    return result;
}
async function revokeSessionById(sessionId, reason) {
    const session = await db_1.prisma.session.findUnique({
        where: { id: sessionId },
        select: { account: true },
    });
    return session ? revokeSession(sessionId, session.account, reason) : false;
}
async function refreshSession(presentedToken, secret) {
    if (!presentedToken || typeof presentedToken !== "string") {
        throw new SessionError("invalid_refresh_token");
    }
    const tokenHash = hashRefreshToken(presentedToken);
    const current = await db_1.prisma.refreshToken.findUnique({
        where: { id: tokenHash },
        include: { session: true },
    });
    if (!current) {
        throw new SessionError("invalid_refresh_token");
    }
    if (current.rotatedAt) {
        await revokeFamily(current.sessionId, "refresh_token_reused");
        throw new SessionError("refresh_token_reused");
    }
    if (current.revokedAt || current.session.revokedAt) {
        throw new SessionError("invalid_refresh_token");
    }
    const nextToken = newRefreshToken();
    const nextHash = hashRefreshToken(nextToken);
    const now = new Date();
    const rotated = await db_1.prisma.$transaction(async (tx) => {
        const stillActive = await tx.session.findUnique({
            where: { id: current.sessionId },
            select: { revokedAt: true },
        });
        if (!stillActive || stillActive.revokedAt) {
            return false;
        }
        const claimed = await tx.refreshToken.updateMany({
            where: {
                id: tokenHash,
                revokedAt: null,
                rotatedAt: null,
                session: { is: { revokedAt: null } },
            },
            data: { revokedAt: now, rotatedAt: now, replacedBy: nextHash },
        });
        if (claimed.count !== 1) {
            return false;
        }
        await tx.refreshToken.create({
            data: {
                id: nextHash,
                sessionId: current.sessionId,
                role: current.role,
                createdAt: now,
            },
        });
        return true;
    });
    if (!rotated) {
        const latest = await db_1.prisma.refreshToken.findUnique({
            where: { id: tokenHash },
            select: { rotatedAt: true, sessionId: true },
        });
        if (latest?.rotatedAt) {
            await revokeFamily(latest.sessionId, "refresh_token_reused");
            throw new SessionError("refresh_token_reused");
        }
        throw new SessionError("invalid_refresh_token");
    }
    return {
        accessToken: issueAccessToken({
            sub: current.session.account,
            role: current.role,
            sid: current.sessionId,
        }, secret),
        refreshToken: nextToken,
    };
}

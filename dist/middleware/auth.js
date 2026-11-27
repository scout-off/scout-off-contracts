"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.isTokenRevoked = isTokenRevoked;
exports.requireAuth = requireAuth;
exports.requireRole = requireRole;
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const client_1 = require("@prisma/client");
const prisma = new client_1.PrismaClient();
async function isTokenRevoked(jti) {
    if (!jti) {
        return false;
    }
    const existingRevocation = await prisma.revoked_tokens.findFirst({
        where: { jti },
    });
    return !!existingRevocation;
}
function requireAuth(req, res, next) {
    const authHeader = req.headers["authorization"];
    if (!authHeader || !authHeader.startsWith("Bearer ")) {
        return res.status(401).json({ error: "Unauthorized - no token provided" });
    }
    const token = authHeader.split(" ")[1];
    try {
        const payload = jsonwebtoken_1.default.verify(token, process.env.JWT_SECRET);
        if (!payload.jti) {
            return res.status(401).json({ error: "Unauthorized - token has no jti claim" });
        }
        const isRevoked = await isTokenRevoked(payload.jti);
        if (isRevoked) {
            return res.status(401).json({ error: "Unauthorized - token has been revoked" });
        }
        req.user = {
            sub: payload.sub,
            role: payload.role,
            jti: payload.jti,
        };
        next();
    }
    catch (error) {
        return res.status(401).json({ error: "Unauthorized - invalid token" });
    }
}
function requireRole(allowedRoles) {
    return function (req, res, next) {
        if (!req.user || !req.user.role) {
            return res.status(401).json({ error: "Unauthorized - no user role" });
        }
        if (!allowedRoles.includes(req.user.role)) {
            return res.status(403).json({ error: "Forbidden - insufficient role" });
        }
        next();
    };
}

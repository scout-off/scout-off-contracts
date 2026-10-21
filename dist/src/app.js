"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.app = void 0;
const express_1 = __importDefault(require("express"));
const client_1 = require("@prisma/client");
const sep10_1 = require("./services/sep10");
const auth_1 = require("./middleware/auth");
const prisma = new client_1.PrismaClient();
const app = (0, express_1.default)();
exports.app = app;
app.use(express_1.default.json());
// In-memory token store for demo (replace with DB in production)
const issuedTokens = new Map();
// SEP-10 token issuance endpoint
app.post("/api/sep10/token", (req, res) => {
    const { sub, role } = req.body;
    if (!sub || !role) {
        return res.status(400).json({ error: "sub and role are required" });
    }
    const token = (0, sep10_1.issueSep10Token)({ sub, role }, process.env.JWT_SECRET);
    issuedTokens.set(token, "active");
    res.json({ token });
});
// Revoke token endpoint (admin)
app.post("/api/admin/tokens/revoke", (0, auth_1.requireRole)(["admin"]), async (req, res) => {
    const { token } = req.body;
    if (!token) {
        return res.status(400).json({ error: "Token is required" });
    }
    let jti;
    try {
        const payload = jwt.verify(token, process.env.JWT_SECRET);
        jti = payload.jti;
    }
    catch {
        return res.status(400).json({ error: "Invalid token" });
    }
    if (!jti) {
        return res.status(400).json({ error: "Token does not contain a jti claim" });
    }
    await prisma.revoked_tokens.create({
        data: { jti },
    });
    // Remove from active tokens
    issuedTokens.delete(token);
    res.json({ revoked: true });
});
// Protected route example
app.get("/api/tokens/me", auth_1.requireAuth, (req, res) => {
    const { sub, role, jti } = req.user;
    const isRevoked = await prisma.revoked_tokens.findFirst({ where: { jti } });
    if (isRevoked) {
        return res.status(401).json({ error: "Token has been revoked" });
    }
    res.json({ sub, role, jti });
});

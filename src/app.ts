import express, { Request, Response } from "express";
import jwt from "jsonwebtoken";
import { issueSep10Token } from "./services/sep10";
import { requireAuth, requireRole } from "./middleware/auth";
import { prisma } from "./db";

const app = express();
app.use(express.json());

// SEP-10 token issuance endpoint
app.post("/api/sep10/token", (req: Request, res: Response) => {
  const { sub, role } = req.body;
  if (!sub || !role) {
    return res.status(400).json({ error: "sub and role are required" });
  }

  const token = issueSep10Token({ sub, role }, process.env.JWT_SECRET!);
  return res.json({ token });
});

// Revoke token endpoint (admin only) — requireAuth must precede requireRole
// so req.user is populated before the role check runs.
app.post(
  "/api/admin/tokens/revoke",
  requireAuth,
  requireRole(["admin"]),
  async (req: Request, res: Response) => {
    const { token } = req.body;

    if (!token) {
      return res.status(400).json({ error: "Token is required" });
    }

    let jti: string;

    try {
      const payload = jwt.verify(token, process.env.JWT_SECRET!) as {
        jti?: string;
      };
      if (!payload.jti) {
        return res
          .status(400)
          .json({ error: "Token does not contain a jti claim" });
      }
      jti = payload.jti;
    } catch {
      return res.status(400).json({ error: "Invalid token" });
    }

    await prisma.revokedToken.create({
      data: { jti },
    });

    return res.json({ revoked: true });
  }
);

// Protected route — returns the authenticated caller's token claims.
app.get("/api/tokens/me", requireAuth, async (req: Request, res: Response) => {
  const { sub, role, jti } = req.user!;

  const isRevoked = await prisma.revokedToken.findFirst({ where: { jti } });

  if (isRevoked) {
    return res.status(401).json({ error: "Token has been revoked" });
  }

  return res.json({ sub, role, jti });
});

export { app };

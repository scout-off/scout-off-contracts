import express, { Request, Response, NextFunction } from "express";
import { PrismaClient } from "@prisma/client";
import { issueSep10Token } from "./services/sep10";
import { requireAuth, requireRole } from "./middleware/auth";

const prisma = new PrismaClient();
const app = express();
app.use(express.json());

// In-memory token store for demo (replace with DB in production)
const issuedTokens = new Map<string, string>();

// SEP-10 token issuance endpoint
app.post("/api/sep10/token", (req: Request, res: Response) => {
  const { sub, role } = req.body;
  if (!sub || !role) {
    return res.status(400).json({ error: "sub and role are required" });
  }

  const token = issueSep10Token({ sub, role }, process.env.JWT_SECRET!);
  issuedTokens.set(token, "active");

  res.json({ token });
});

// Revoke token endpoint (admin)
app.post("/api/admin/tokens/revoke", requireRole(["admin"]), async (req: Request, res: Response) => {
  const { token } = req.body;

  if (!token) {
    return res.status(400).json({ error: "Token is required" });
  }

  let jti: string;

  try {
    const payload: any = jwt.verify(token, process.env.JWT_SECRET!);
    jti = payload.jti;
  } catch {
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
app.get("/api/tokens/me", requireAuth, (req: Request, res: Response) => {
  const { sub, role, jti } = req.user!;

  const isRevoked = await prisma.revoked_tokens.findFirst({ where: { jti } });

  if (isRevoked) {
    return res.status(401).json({ error: "Token has been revoked" });
  }

  res.json({ sub, role, jti });
});

export { app };
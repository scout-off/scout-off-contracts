import express, { Request, Response, NextFunction } from "express";
import jwt from "jsonwebtoken";
import {
  buildChallengeTx,
  verifyChallengeTx,
  issueSession,
  refreshSession,
} from "./services/sep10";
import { requireAuth, requireRole } from "./middleware/auth";
import { prisma } from "./db";

const app = express();
app.use(express.json());

// SEP-10 challenge endpoint — returns a server-signed challenge
// transaction that the client must sign and return.
app.get("/auth", async (req: Request, res: Response) => {
  const account = req.query.account as string | undefined;
  if (!account) {
    return res.status(400).json({ error: "account query parameter is required" });
  }

  try {
    const challenge = await buildChallengeTx(account);
    res.json({ challenge });
  } catch (error) {
    return res.status(500).json({
      error: error instanceof Error ? error.message : "Internal server error",
    });
  }
});

// SEP-10 verification endpoint — client submits the signed challenge
// transaction; server verifies it and issues a JWT whose role is
// derived from on-chain / indexer state.
app.post("/auth", async (req: Request, res: Response) => {
  const { transaction, account } = req.body as {
    transaction?: string;
    account?: string;
  };

  if (!transaction || !account) {
    return res
      .status(400)
      .json({ error: "transaction and account are required" });
  }

  try {
    const verifiedAccount = await verifyChallengeTx(transaction, account);
    const session = await issueSession(verifiedAccount);

    res.json(session);
  } catch (error) {
    return res.status(401).json({
      error: error instanceof Error ? error.message : "Authentication failed",
    });
  }
});

// Refresh endpoint — rotates the refresh token and re-derives the role.
app.post("/auth/refresh", async (req: Request, res: Response) => {
  const { refreshToken } = req.body as { refreshToken?: string };

  if (!refreshToken) {
    return res.status(400).json({ error: "refreshToken is required" });
  }

  try {
    res.json(await refreshSession(refreshToken));
  } catch (error) {
    return res.status(401).json({
      error: error instanceof Error ? error.message : "Authentication failed",
    });
  }
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

import express, { Request, Response, NextFunction } from "express";
import jwt from "jsonwebtoken";
import { prisma } from "./db";
import {
  buildChallengeTx,
  verifyChallengeTx,
  deriveRole,
  issueSep10Token,
} from "./services/sep10";
import { requireAuth, requireRole } from "./middleware/auth";

const app = express();
app.use(express.json());

// In-memory token store for demo (replace with DB in production)
const issuedTokens = new Map<string, string>();

// SEP-10 challenge endpoint — returns a server-signed challenge
// transaction that the client must sign and return.
app.get("/auth", (req: Request, res: Response) => {
  const account = req.query.account as string | undefined;
  if (!account) {
    return res.status(400).json({ error: "account query parameter is required" });
  }

  try {
    const challenge = buildChallengeTx(account);
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
    const role = await deriveRole(verifiedAccount);
    const token = issueSep10Token({ sub: verifiedAccount, role }, process.env.JWT_SECRET!);

    issuedTokens.set(token, "active");

    res.json({ token });
  } catch (error) {
    return res.status(401).json({
      error: error instanceof Error ? error.message : "Authentication failed",
    });
  }
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

  await prisma.revokedToken.create({
    data: { jti },
  });

  // Remove from active tokens
  issuedTokens.delete(token);

  res.json({ revoked: true });
});

// Protected route example
app.get("/api/tokens/me", requireAuth, async (req: Request, res: Response) => {
  const { sub, role, jti } = req.user!;

  const isRevoked = await prisma.revokedToken.findFirst({ where: { jti } });

  if (isRevoked) {
    return res.status(401).json({ error: "Token has been revoked" });
  }

  res.json({ sub, role, jti });
});

export { app };

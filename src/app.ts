import express, { Request, Response, NextFunction } from "express";
import jwt from "jsonwebtoken";
import { StrKey } from "@stellar/stellar-sdk";
import { config } from "./config";
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

function isValidAccount(account: unknown): account is string {
  return typeof account === "string" && StrKey.isValidEd25519PublicKey(account);
}

/**
 * Map a SEP-10 verification failure to a stable error code so raw SDK /
 * exception messages never reach the client. Returns `null` for errors that
 * are not verification failures (e.g. database outages).
 */
function classifyAuthError(error: unknown): string | null {
  const message = error instanceof Error ? error.message : "";
  const name = error instanceof Error ? error.name : "";
  if (/expired|time ?bounds/i.test(message)) return "challenge_expired";
  if (/already been used/i.test(message)) return "challenge_replayed";
  if (/account mismatch/i.test(message)) return "account_mismatch";
  if (/signature|signer/i.test(message)) return "bad_signature";
  if (name === "InvalidChallengeError" || /challenge/i.test(message)) {
    return "invalid_challenge";
  }
  return null;
}

// SEP-10 challenge endpoint — returns a server-signed challenge
// transaction that the client must sign and return.
app.get("/auth", async (req: Request, res: Response) => {
  const account = req.query.account;
  if (account === undefined || account === "") {
    return res.status(400).json({ error: "account query parameter is required" });
  }
  if (!isValidAccount(account)) {
    return res.status(400).json({ error: "invalid account" });
  }

  try {
    const challenge = await buildChallengeTx(account);
    res.json({ challenge });
  } catch (error) {
    console.error("GET /auth: failed to build challenge", error);
    return res.status(500).json({ error: "Internal server error" });
  }
});

// SEP-10 verification endpoint — client submits the signed challenge
// transaction; server verifies it and issues an access token plus a rotating
// refresh token, with the role derived from on-chain / indexer state.
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
  if (typeof transaction !== "string" || !isValidAccount(account)) {
    return res.status(400).json({ error: "invalid account" });
  }

  try {
    const verifiedAccount = await verifyChallengeTx(transaction, account);
    const session = await issueSession(verifiedAccount);

    res.json(session);
  } catch (error) {
    const code = classifyAuthError(error);
    if (code === null) {
      console.error("POST /auth: unexpected error", error);
      return res.status(500).json({ error: "Internal server error" });
    }
    console.warn(`POST /auth: verification failed (${code})`, error);
    return res.status(401).json({ error: code });
  }
});

// Refresh endpoint — rotates the refresh token and re-derives the role.
app.post("/auth/refresh", async (req: Request, res: Response) => {
  const { refreshToken } = req.body as { refreshToken?: string };

  if (!refreshToken || typeof refreshToken !== "string") {
    return res.status(400).json({ error: "refreshToken is required" });
  }

  try {
    res.json(await refreshSession(refreshToken));
  } catch (error) {
    if (error instanceof Error && error.message === "Invalid refresh token") {
      return res.status(401).json({ error: "invalid_refresh_token" });
    }
    console.error("POST /auth/refresh: unexpected error", error);
    return res.status(500).json({ error: "Internal server error" });
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
      const payload = jwt.verify(token, config.jwtSecret, { algorithms: ["HS256"] }) as {
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

import express, { Request, Response, NextFunction } from "express";
import jwt from "jsonwebtoken";
import { StrKey } from "@stellar/stellar-sdk";
import { config } from "./config";
import {
  buildChallengeTx,
  verifyChallengeTx,
  deriveRole,
  issueSep10Token,
} from "./services/sep10";
import { requireAuth, requireRole } from "./middleware/auth";
import { prisma } from "./db";

const app = express();
app.use(express.json());

// In-memory token store for demo (replace with DB in production)
const issuedTokens = new Map<string, string>();

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
app.get("/auth", (req: Request, res: Response) => {
  const account = req.query.account;
  if (account === undefined || account === "") {
    return res.status(400).json({ error: "account query parameter is required" });
  }
  if (!isValidAccount(account)) {
    return res.status(400).json({ error: "invalid account" });
  }

  try {
    const challenge = buildChallengeTx(account);
    res.json({ challenge });
  } catch (error) {
    console.error("GET /auth: failed to build challenge", error);
    return res.status(500).json({ error: "Internal server error" });
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
  if (typeof transaction !== "string" || !isValidAccount(account)) {
    return res.status(400).json({ error: "invalid account" });
  }

  try {
    const verifiedAccount = await verifyChallengeTx(transaction, account);
    const role = await deriveRole(verifiedAccount);
    const token = issueSep10Token({ sub: verifiedAccount, role }, config.jwtSecret);

    issuedTokens.set(token, "active");

    res.json({ token });
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

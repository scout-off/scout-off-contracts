import express, { Request, Response, NextFunction } from "express";
import { jwtVerify as joseVerify } from "jose";
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
import { buildJwks } from "./jwks";

const app = express();
app.use(express.json());

// JWKS endpoint – exposes only public keys so verifiers can rotate without
// sharing secrets. max-age = max access token lifetime (15 min).
app.get("/.well-known/jwks.json", (_req: Request, res: Response) => {
  if (!config.signingKeys) {
    return res.status(503).json({ error: "JWKS not available in HS256 migration mode" });
  }
  res
    .set("Cache-Control", "public, max-age=900")
    .json(buildJwks(config.signingKeys));
});

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
      // Use the same asymmetric / migration verification logic as requireAuth.
      const { signingKeys, allowHs256Migration, jwtSecret } = config;
      let jtiValue: string | undefined;

      if (signingKeys) {
        const parts = token.split(".");
        if (parts.length !== 3) throw new Error("malformed");
        const header = JSON.parse(Buffer.from(parts[0], "base64url").toString());
        const kid = typeof header.kid === "string" ? header.kid : undefined;
        const pubKey = kid ? signingKeys.publicKeys.get(kid) : undefined;
        if (!pubKey) return res.status(400).json({ error: "Invalid token" });
        const { payload } = await joseVerify(token, pubKey, { algorithms: ["EdDSA"] });
        jtiValue = payload.jti as string | undefined;
      } else if (allowHs256Migration) {
        // eslint-disable-next-line @typescript-eslint/no-require-imports
        const jwt = require("jsonwebtoken") as typeof import("jsonwebtoken");
        const payload = jwt.verify(token, jwtSecret, { algorithms: ["HS256"] }) as {
          jti?: string;
        };
        jtiValue = payload.jti;
      } else {
        return res.status(500).json({ error: "No verification key available" });
      }

      if (!jtiValue) {
        return res.status(400).json({ error: "Token does not contain a jti claim" });
      }
      jti = jtiValue;
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

  return res.json({ sub, role, jti });
});

export { app };

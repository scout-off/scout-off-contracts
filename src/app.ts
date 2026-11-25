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
import { initRateLimiting, checkChallengeLimit } from "./middleware/rateLimit";
import { prisma } from "./db";
import { logger, generateRequestId, attachRequestIdToResponse } from "./logger";
import pinoHttp from "pino-http";
import { metricsAuthMiddleware } from "./middleware/metricsAuth";
import {
  httpRequestDuration,
  getRouteLabel,
  authChallengesIssued,
  authVerifications,
  authRefreshes,
  tokensRevoked,
  register,
} from "./metrics";
import { buildJwks } from "./jwks";

const app = express();

// Health endpoints are registered before the JSON body parser and any
// auth/rate-limiting/logging middleware so they stay unauthenticated and
// generate no logging noise.
app.get("/healthz", (_req: Request, res: Response) => {
  res.status(200).json({ status: "ok" });
});

app.get("/readyz", async (_req: Request, res: Response) => {
  const timeoutMs = 2000;
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error("readiness timeout")), timeoutMs);
  });

  try {
    await Promise.race([prisma.$queryRaw`SELECT 1`, timeout]);
    res.status(200).json({ status: "ok" });
  } catch (error) {
    logger.error(
      { error: error instanceof Error ? error.message : String(error) },
      "GET /readyz: readiness check failed"
    );
    res.status(503).json({ status: "unavailable" });
  } finally {
    if (timer) clearTimeout(timer);
  }
});

// Pino HTTP middleware for automatic request/response logging
const pinoMiddleware = pinoHttp({
  logger,
  customLogLevel: (req, res, err) => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    if ((res as any).statusCode >= 500 || err) return "error";
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    if ((res as any).statusCode >= 400) return "warn";
    return "info";
  },
  customSuccessObject: (req, res) => ({
    method: (req as Request).method,
    url: (req as Request).url,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    statusCode: (res as any).statusCode,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    duration: (res as any).responseTime,
  }),
  customReceivedObject: (req) => ({
    method: (req as Request).method,
    url: (req as Request).url,
  }),
  customErrorMessage: (err, req, res) => {
    // pino-http types are incorrect, so we cast to unknown first
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const request = req as any;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const response = res as any;
    return `${request.method || "UNKNOWN"} ${request.url || "UNKNOWN"} failed with status ${response.statusCode || "UNKNOWN"}: ${err instanceof Error ? err.message : String(err)}`;
  },
  // Redact sensitive fields from request/response
  redact: {
    paths: [
      "req.headers.authorization",
      "*.transaction",
      "*.refreshToken",
      "*.token",
      "error",
      "*.error",
    ],
    censor: "[REDACTED]",
  },
});

app.use(pinoMiddleware);
app.use(express.json());

// Metrics endpoint must come before other routes to avoid being captured by request duration middleware
app.use("/metrics", metricsAuthMiddleware);
app.get("/metrics", async (req: Request, res: Response) => {
  res.set("Content-Type", register.contentType);
  res.send(await register.metrics());
});

// Request duration tracking middleware
app.use((req: Request, res: Response, next: NextFunction) => {
  const end = httpRequestDuration.labels(req.method, getRouteLabel(req.url), "unknown").startTimer();
  res.on("finish", () => {
    end({ status: (res as any).statusCode.toString() });
  });
  next();
});

// Manual request ID middleware to ensure consistent x-request-id header
app.use((req: Request, res: Response, next: NextFunction) => {
  (req as Request & { requestId?: string }).requestId = generateRequestId(req);
  attachRequestIdToResponse(req, res);
  next();
});

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

// Initialize rate limiting middleware
const rateLimitMiddleware = initRateLimiting(app);

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
app.get(
  "/auth",
  rateLimitMiddleware.getAuth,
  rateLimitMiddleware.getAuthAccount,
  async (req: Request, res: Response) => {
  const requestId = (req as Request & { requestId?: string }).requestId;
  const account = req.query.account;
  if (account === undefined || account === "") {
    logger.warn({ requestId }, "GET /auth: missing account parameter");
    return res.status(400).json({ error: "account query parameter is required" });
  }
  if (!isValidAccount(account)) {
    logger.warn({ requestId }, "GET /auth: invalid account format", {
      account,
    });
    return res.status(400).json({ error: "invalid account" });
  }

  try {
    const challenge = await buildChallengeTx(account);
    logger.info({ requestId, account }, "GET /auth: challenge generated");
    authChallengesIssued.inc();
    res.json({ challenge });
  } catch (error) {
    logger.error(
      { requestId, account, error: error instanceof Error ? error.message : String(error) },
      "GET /auth: failed to build challenge"
    );
    return res.status(500).json({ error: "Internal server error" });
  }
});

// SEP-10 verification endpoint — client submits the signed challenge
// transaction; server verifies it and issues an access token plus a rotating
// refresh token, with the role derived from on-chain / indexer state.
app.post(
  "/auth",
  rateLimitMiddleware.postAuth,
  async (req: Request, res: Response) => {
  const requestId = (req as Request & { requestId?: string }).requestId;
  const { transaction, account } = req.body as {
    transaction?: string;
    account?: string;
  };

  if (!transaction || !account) {
    logger.warn({ requestId }, "POST /auth: missing transaction or account");
    return res
      .status(400)
      .json({ error: "transaction and account are required" });
  }
  if (typeof transaction !== "string") {
    logger.warn({ requestId }, "POST /auth: invalid transaction");
    return res.status(400).json({ error: "invalid transaction" });
  }
  if (!isValidAccount(account)) {
    logger.warn({ requestId, account }, "POST /auth: invalid account");
    return res.status(400).json({ error: "invalid account" });
  }

  try {
    const verifiedAccount = await verifyChallengeTx(transaction, account);
    
    // Cap outstanding unconsumed challenges per account
    await checkChallengeLimit(account, config.maxUnconsumedChallengesPerAccount);
    
    const session = await issueSession(verifiedAccount);

    logger.info(
      { requestId, account: verifiedAccount, jti: session.token.split(".")[1] },
      "POST /auth: session issued"
    );
    authVerifications.labels("ok").inc();
    res.json(session);
  } catch (error) {
    const code = classifyAuthError(error);
    if (code === null) {
      logger.error(
        { requestId, error: error instanceof Error ? error.message : String(error) },
        "POST /auth: unexpected error"
      );
      return res.status(500).json({ error: "Internal server error" });
    }
    logger.warn({ requestId, code }, `POST /auth: verification failed (${code})`);
    authVerifications.labels(code).inc();
    return res.status(401).json({ error: code });
  }
});

// Refresh endpoint — rotates the refresh token and re-derives the role.
app.post("/auth/refresh", rateLimitMiddleware.postAuthRefresh, async (req: Request, res: Response) => {
  const requestId = (req as Request & { requestId?: string }).requestId;
  const { refreshToken } = req.body as { refreshToken?: string };

  if (!refreshToken || typeof refreshToken !== "string") {
    logger.warn({ requestId }, "POST /auth/refresh: missing refreshToken");
    return res.status(400).json({ error: "refreshToken is required" });
  }

  try {
    const session = await refreshSession(refreshToken);
    logger.info(
      { requestId, account: session.token.split(".")[1] },
      "POST /auth/refresh: session refreshed"
    );
    authRefreshes.labels("ok").inc();
    res.json(session);
  } catch (error) {
    if (error instanceof Error && error.message === "Invalid refresh token") {
      logger.warn({ requestId }, "POST /auth/refresh: invalid refresh token");
      authRefreshes.labels("invalid").inc();
      return res.status(401).json({ error: "invalid_refresh_token" });
    }
    logger.error(
      { requestId, error: error instanceof Error ? error.message : String(error) },
      "POST /auth/refresh: unexpected error"
    );
    authRefreshes.labels("error").inc();
    return res.status(500).json({ error: "Internal server error" });
  }
});

// Revoke token endpoint (admin only) — requireAuth must precede requireRole
// so req.user is populated before the role check runs.
// Rate limiting is applied to all admin endpoints.
app.post(
  "/api/admin/tokens/revoke",
  rateLimitMiddleware.admin,
  requireAuth,
  requireRole(["admin"]),
  async (req: Request, res: Response) => {
    const requestId = (req as Request & { requestId?: string }).requestId;
    const { token } = req.body;

    if (!token) {
      logger.warn({ requestId }, "POST /api/admin/tokens/revoke: missing token");
      return res.status(400).json({ error: "Token is required" });
    }

    let jti: string;
    let exp: number | null = null;

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
        const { payload } = await joseVerify(token, pubKey, {
          algorithms: ["EdDSA"],
          issuer: config.jwtIssuer || undefined,
          audience: config.jwtAudience || undefined,
        });
        jtiValue = payload.jti as string | undefined;
        exp = typeof payload.exp === "number" ? payload.exp : null;
      } else if (allowHs256Migration) {
        // eslint-disable-next-line @typescript-eslint/no-require-imports
        const jwt = require("jsonwebtoken") as typeof import("jsonwebtoken");
        const payload = jwt.verify(token, jwtSecret, {
          algorithms: ["HS256"],
          issuer: config.jwtIssuer || undefined,
          audience: config.jwtAudience || undefined,
        }) as {
          jti?: string;
          exp?: number;
        };
        jtiValue = payload.jti;
        exp = payload.exp ?? null;
      } else {
        return res.status(500).json({ error: "No verification key available" });
      }

      if (!jtiValue) {
        return res.status(400).json({ error: "Token does not contain a jti claim" });
      }
      jti = jtiValue;
    } catch {
      logger.warn({ requestId }, "POST /api/admin/tokens/revoke: invalid token");
      return res.status(400).json({ error: "Invalid token" });
    }

    // Store the token expiry so cleanup can properly track when to delete the revocation
    await prisma.revokedToken.create({
      data: {
        jti,
        expiresAt: exp ? new Date(exp * 1000) : undefined,
      },
    });

    logger.info({ requestId, jti }, "POST /api/admin/tokens/revoke: token revoked");
    tokensRevoked.inc();
    return res.json({ revoked: true });
  }
);

// Protected route — returns the authenticated caller's token claims.
app.get("/api/tokens/me", requireAuth, async (req: Request, res: Response) => {
  const requestId = (req as Request & { requestId?: string }).requestId;
  const { sub, role, jti } = req.user!;

  logger.info(
    { requestId, sub, role, jti },
    "GET /api/tokens/me: token claims returned"
  );
  return res.json({ sub, role, jti });
});

export { app };

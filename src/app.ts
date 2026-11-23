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

const app = express();

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
app.post("/auth", async (req: Request, res: Response) => {
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
  if (typeof transaction !== "string" || !isValidAccount(account)) {
    logger.warn({ requestId, account }, "POST /auth: invalid account");
    return res.status(400).json({ error: "invalid account" });
  }

  try {
    const verifiedAccount = await verifyChallengeTx(transaction, account);
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
app.post("/auth/refresh", async (req: Request, res: Response) => {
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
app.post(
  "/api/admin/tokens/revoke",
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

    try {
      const payload = jwt.verify(token, config.jwtSecret, { algorithms: ["HS256"] }) as {
        jti?: string;
      };
      if (!payload.jti) {
        logger.warn(
          { requestId },
          "POST /api/admin/tokens/revoke: token does not contain jti claim"
        );
        return res
          .status(400)
          .json({ error: "Token does not contain a jti claim" });
      }
      jti = payload.jti;
    } catch {
      logger.warn({ requestId }, "POST /api/admin/tokens/revoke: invalid token");
      return res.status(400).json({ error: "Invalid token" });
    }

    await prisma.revokedToken.create({
      data: { jti },
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

  const isRevoked = await prisma.revokedToken.findFirst({ where: { jti } });

  if (isRevoked) {
    logger.warn({ requestId, jti }, "GET /api/tokens/me: token has been revoked");
    return res.status(401).json({ error: "Token has been revoked" });
  }

  logger.info(
    { requestId, sub, role, jti },
    "GET /api/tokens/me: token claims returned"
  );
  return res.json({ sub, role, jti });
});

export { app };

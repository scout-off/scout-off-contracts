/**
 * src/middleware/auth.ts
 *
 * Express middleware for authenticating requests via SEP-10 JWT.
 *
 * Usage:
 *   router.get("/protected", requireAuth, (req, res) => { ... });
 *
 * The middleware:
 *   1. Extracts the Bearer token from the Authorization header.
 *   2. Verifies the JWT signature using the configured JWT_SECRET.
 *   3. Validates that `iss` matches JWT_ISSUER and `aud` matches JWT_AUDIENCE.
 *   4. Validates that the token has not expired.
 *   5. Sets `req.stellarAddress` to the authenticated Stellar account (sub claim).
 *
 * On failure, responds with 401 Unauthorized and a JSON error body.
 *
 * @see src/services/sep10.ts for JWT issuance and configuration.
 */

import { Request, Response, NextFunction } from "express";
import { JsonWebTokenError, TokenExpiredError } from "jsonwebtoken";
import { verifySep10Token } from "../services/sep10.js";

// ---------------------------------------------------------------------------
// Augment Express Request type with the authenticated Stellar address
// ---------------------------------------------------------------------------

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace
  namespace Express {
    interface Request {
      /** Authenticated Stellar G-address extracted from the JWT `sub` claim. */
      stellarAddress?: string;
    }
  }
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/**
 * Express middleware that enforces JWT authentication.
 *
 * Validates signature, expiry, issuer, and audience. Sets `req.stellarAddress`
 * on success. Returns 401 on any validation failure.
 *
 * @example
 * ```ts
 * import { requireAuth } from "./middleware/auth.js";
 *
 * router.get("/scouts/me", requireAuth, (req, res) => {
 *   res.json({ address: req.stellarAddress });
 * });
 * ```
 */
export function requireAuth(
  req: Request,
  res: Response,
  next: NextFunction
): void {
  const authHeader = req.headers.authorization;

  if (!authHeader || !authHeader.startsWith("Bearer ")) {
    res.status(401).json({
      error: "Unauthorized",
      message: "Missing or malformed Authorization header (expected: Bearer <token>)",
    });
    return;
  }

  const token = authHeader.slice("Bearer ".length).trim();

  try {
    const payload = verifySep10Token(token);
    req.stellarAddress = payload.sub;
    next();
  } catch (err) {
    if (err instanceof TokenExpiredError) {
      res.status(401).json({
        error: "Unauthorized",
        message: "Token has expired — please re-authenticate via SEP-10",
      });
      return;
    }

    if (err instanceof JsonWebTokenError) {
      // Covers wrong issuer, wrong audience, bad signature, malformed token
      res.status(401).json({
        error: "Unauthorized",
        message: "Invalid token — signature, issuer, or audience mismatch",
      });
      return;
    }

    // Unexpected error — let the global error handler deal with it
    next(err);
  }
}

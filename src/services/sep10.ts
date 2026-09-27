/**
 * src/services/sep10.ts
 *
 * SEP-10 JWT issuance service.
 *
 * Reads JWT configuration from environment variables with sensible defaults:
 *
 *   JWT_SECRET      — HMAC-SHA256 signing secret (required in production)
 *   JWT_TTL_SECONDS — Token lifetime in seconds (default: 86400 = 24 h)
 *   JWT_ISSUER      — `iss` claim value (default: "scoutchain-backend")
 *   JWT_AUDIENCE    — `aud` claim value (default: "scoutchain-api")
 *
 * SEP-10 guidance: the `sub` claim MUST be the authenticated Stellar account
 * (G-address). Tokens are short-lived (default 24 h; previous default was
 * 7 d — see issue #1473).
 *
 * @see https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0010.md
 */

import jwt, { SignOptions, JwtPayload } from "jsonwebtoken";

// ---------------------------------------------------------------------------
// Configuration — read once at module load time
// ---------------------------------------------------------------------------

/** Signing secret. Must be set in production via JWT_SECRET. */
const JWT_SECRET: string = process.env.JWT_SECRET ?? "dev-insecure-secret";

/**
 * Token lifetime in seconds.
 *
 * Default: 86400 (24 hours).
 * SEP-10 recommends short-lived tokens; avoid values above 86400 in production.
 */
const JWT_TTL_SECONDS: number = (() => {
  const raw = process.env.JWT_TTL_SECONDS;
  if (!raw) return 86_400; // 24 hours
  const parsed = parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(
      `Invalid JWT_TTL_SECONDS="${raw}": must be a positive integer (seconds)`
    );
  }
  return parsed;
})();

/**
 * Issuer claim (`iss`). Identifies this backend service.
 * Default: "scoutchain-backend".
 */
const JWT_ISSUER: string = process.env.JWT_ISSUER ?? "scoutchain-backend";

/**
 * Audience claim (`aud`). Identifies the intended API consumer.
 * Default: "scoutchain-api".
 */
const JWT_AUDIENCE: string = process.env.JWT_AUDIENCE ?? "scoutchain-api";

// ---------------------------------------------------------------------------
// Exported config accessor (useful for tests and requireAuth middleware)
// ---------------------------------------------------------------------------

export interface JwtConfig {
  secret: string;
  ttlSeconds: number;
  issuer: string;
  audience: string;
}

/**
 * Returns the active JWT configuration derived from environment variables.
 * Callers should cache the result rather than calling this on every request.
 */
export function getJwtConfig(): JwtConfig {
  return {
    secret: JWT_SECRET,
    ttlSeconds: JWT_TTL_SECONDS,
    issuer: JWT_ISSUER,
    audience: JWT_AUDIENCE,
  };
}

// ---------------------------------------------------------------------------
// Token issuance
// ---------------------------------------------------------------------------

export interface Sep10TokenPayload {
  /** Authenticated Stellar account G-address (SEP-10 requirement). */
  sub: string;
  /** Issuer — identifies this backend service. */
  iss: string;
  /** Audience — identifies the intended API consumer. */
  aud: string;
  /** Issued-at timestamp (seconds since epoch). */
  iat: number;
  /** Expiry timestamp (seconds since epoch). */
  exp: number;
}

/**
 * Issues a signed JWT for a successfully SEP-10 authenticated Stellar account.
 *
 * @param stellarAddress - The authenticated Stellar G-address. Becomes the `sub` claim.
 * @returns A signed JWT string.
 *
 * @example
 * ```ts
 * const token = issueSep10Token("GABC...XYZ");
 * res.json({ token });
 * ```
 */
export function issueSep10Token(stellarAddress: string): string {
  if (!stellarAddress || !stellarAddress.startsWith("G")) {
    throw new Error(
      `issueSep10Token: stellarAddress must be a valid Stellar G-address, got "${stellarAddress}"`
    );
  }

  const config = getJwtConfig();

  const options: SignOptions = {
    expiresIn: config.ttlSeconds,
    issuer: config.issuer,
    audience: config.audience,
  };

  return jwt.sign({ sub: stellarAddress }, config.secret, options);
}

/**
 * Verifies and decodes a JWT issued by this service.
 *
 * Validates signature, expiry, issuer, and audience.
 *
 * @param token - The raw JWT string from the Authorization header.
 * @returns The decoded payload.
 * @throws `JsonWebTokenError` | `TokenExpiredError` | `NotBeforeError` on invalid tokens.
 */
export function verifySep10Token(token: string): Sep10TokenPayload {
  const config = getJwtConfig();

  const payload = jwt.verify(token, config.secret, {
    issuer: config.issuer,
    audience: config.audience,
  }) as JwtPayload;

  if (!payload.sub) {
    throw new Error("JWT missing required sub claim");
  }

  return payload as Sep10TokenPayload;
}

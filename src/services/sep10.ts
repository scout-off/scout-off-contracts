import { v4 as uuidv4 } from "uuid";
import { createHash, randomBytes } from "crypto";
import { SignJWT, decodeJwt } from "jose";
import { WebAuth, Keypair, TransactionBuilder } from "@stellar/stellar-sdk";
import { prisma } from "../db";
import { config } from "../config";
import type { SigningKeys } from "../keys";

interface Sep10TokenPayload {
  sub: string;
  role: string;
  /** Optional OAuth-style scope, e.g. `onboarding` for unknown-role wallets. */
  scope?: string;
}

/** Challenge validity window; also used as the WebAuth `timeout`. */
const CHALLENGE_TTL_SECONDS = 300;
/** Hard ceiling on access-token lifetime (#1606). */
const MAX_ACCESS_TOKEN_TTL_SECONDS = 15 * 60;
const DEFAULT_REFRESH_TOKEN_TTL_SECONDS = 7 * 24 * 60 * 60;
/**
 * Refresh lifetime for unregistered wallets (#1821). Short-lived so an
 * `unknown` session cannot be kept alive indefinitely; the wallet must
 * register (or re-authenticate) to obtain a normal session.
 */
const DEFAULT_UNKNOWN_REFRESH_TOKEN_TTL_SECONDS = 60 * 60;
/** Scope claim carried by tokens issued to unregistered wallets. */
export const ONBOARDING_SCOPE = "onboarding";

function sha256Hex(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

/**
 * Signature-independent challenge identifier: the transaction hash, which
 * stays the same when the client adds its signature. 64 hex chars, so it
 * fits the `VARCHAR(64)` key (the full base64 XDR would not).
 */
function challengeHash(tx: { hash(): Buffer }): string {
  return tx.hash().toString("hex");
}

/**
 * Access-token lifetime in seconds, validated and clamped to the 15-minute
 * ceiling (#1606) by `loadConfig`.
 */
export function accessTokenTtlSeconds(): number {
  return config.accessTokenTtlSeconds;
}

export function refreshTokenTtlSeconds(role?: string): number {
  if (role === "unknown") {
    const configured = Number(process.env.UNKNOWN_REFRESH_TOKEN_TTL_SECONDS);
    return Number.isFinite(configured) && configured > 0
      ? Math.floor(configured)
      : DEFAULT_UNKNOWN_REFRESH_TOKEN_TTL_SECONDS;
  }
  const configured = Number(process.env.REFRESH_TOKEN_TTL_SECONDS);
  return Number.isFinite(configured) && configured > 0
    ? Math.floor(configured)
    : DEFAULT_REFRESH_TOKEN_TTL_SECONDS;
}

export async function issueSep10Token(
  payload: Sep10TokenPayload,
  cfg: {
    signingKeys: SigningKeys | null;
    allowHs256Migration: boolean;
    jwtSecret: string;
  }
): Promise<string> {
  const jti = uuidv4();
  const { signingKeys, allowHs256Migration, jwtSecret } = cfg;
  const claims: { sub: string; role: string; scope?: string } = {
    sub: payload.sub,
    role: payload.role,
    ...(payload.scope ? { scope: payload.scope } : {}),
  };

  if (signingKeys) {
    return new SignJWT(claims)
      .setProtectedHeader({ alg: "EdDSA", kid: signingKeys.activeKid })
      .setJti(jti)
      .setIssuedAt()
      .setIssuer(config.jwtIssuer)
      .setAudience(config.jwtAudience)
      .setExpirationTime(Math.floor(Date.now() / 1000) + accessTokenTtlSeconds())
      .sign(signingKeys.activePrivateKey);
  }

  if (allowHs256Migration) {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const jwt = require("jsonwebtoken") as typeof import("jsonwebtoken");
    return jwt.sign(claims, jwtSecret, {
      algorithm: "HS256",
      expiresIn: accessTokenTtlSeconds(),
      jwtid: jti,
      issuer: config.jwtIssuer,
      audience: config.jwtAudience,
    });
  }

  throw new Error("No signing key available: set JWT_SIGNING_KEYS");
}

interface Sep10Config {
  serverKeypair: Keypair;
  homeDomain: string;
  webAuthDomain: string;
  networkPassphrase: string;
}

let cachedSep10Config: Sep10Config | undefined;

/**
 * Lazily memoised SEP-10 configuration. Config is immutable after startup,
 * so the signing Keypair is derived once per process on first use (lazy so
 * tests that mutate env before import still work).
 */
function sep10Config(): Sep10Config {
  if (!cachedSep10Config) {
    cachedSep10Config = {
      serverKeypair: Keypair.fromSecret(config.sep10SigningSecret),
      homeDomain: config.homeDomain,
      webAuthDomain: config.webAuthDomain,
      networkPassphrase: config.networkPassphrase,
    };
  }
  return cachedSep10Config;
}

/**
 * Decode the exp claim from a SEP-10 token without verifying it.
 * Used to store the expiry time in revoked_tokens.
 */
export function decodeTokenExpiry(token: string): number | null {
  try {
    const payload = decodeJwt(token);
    return typeof payload.exp === "number" ? payload.exp : null;
  } catch {
    return null;
  }
}

/**
 * Build a SEP-10 challenge transaction for the given Stellar account and
 * record it in `sep10_challenges` so it can be consumed exactly once.
 *
 * The SDK derives the time bounds from the current server time
 * (`minTime = now`, `maxTime = now + CHALLENGE_TTL_SECONDS`).
 */
export async function buildChallengeTx(account: string): Promise<string> {
  const { serverKeypair, homeDomain, webAuthDomain, networkPassphrase } =
    sep10Config();

  const challenge = WebAuth.buildChallengeTx(
    serverKeypair,
    account,
    homeDomain,
    CHALLENGE_TTL_SECONDS,
    networkPassphrase,
    webAuthDomain
  );

  const challengeTx = TransactionBuilder.fromXDR(challenge, networkPassphrase);

  await prisma.sep10Challenge.create({
    data: {
      hash: challengeHash(challengeTx),
      account,
      expiresAt: new Date(Date.now() + CHALLENGE_TTL_SECONDS * 1000),
    },
  });

  return challenge;
}

/**
 * Verify a client-signed SEP-10 challenge transaction and return the
 * verified Stellar account ID.
 *
 * Throws if the challenge is invalid, expired, unknown, already used, or
 * signed by an unauthorised signer. The challenge is consumed with a single
 * atomic conditional UPDATE, so concurrent submissions of the same signed
 * challenge cannot both succeed.
 */
export async function verifyChallengeTx(
  challengeTx: string,
  clientAccount: string
): Promise<string> {
  const { serverKeypair, homeDomain, webAuthDomain, networkPassphrase } =
    sep10Config();
  const serverAccount = serverKeypair.publicKey();

  // readChallengeTx validates the structure, time bounds, sequence, home
  // domain, web auth domain and network passphrase.
  const { tx, clientAccountID } = WebAuth.readChallengeTx(
    challengeTx,
    serverAccount,
    networkPassphrase,
    homeDomain,
    webAuthDomain
  );

  if (clientAccountID !== clientAccount) {
    throw new AuthError("account_mismatch", "Challenge account mismatch");
  }

  // The client account must have signed the transaction. Done before the
  // challenge is consumed so an unsigned submission cannot burn it.
  wrapSdkAuth(() =>
    WebAuth.verifyChallengeTxSigners(
      challengeTx,
      serverAccount,
      networkPassphrase,
      [clientAccount],
      homeDomain,
      webAuthDomain
    )
  );

  // Replay protection: atomically mark the challenge consumed. Zero rows
  // updated means it is unknown, expired, or already used.
  const now = new Date();
  const { count } = await prisma.sep10Challenge.updateMany({
    where: {
      hash: challengeHash(tx),
      account: clientAccount,
      consumedAt: null,
      expiresAt: { gt: now },
    },
    data: { consumedAt: now },
  });
  if (count !== 1) {
    throw new AuthError("challenge_replayed", "Challenge has already been used");
  }

  return clientAccountID;
}

/**
 * Issue an access token plus a rotating refresh token for `account`, with
 * the role freshly derived from indexer state.
 *
 * Unregistered wallets (`unknown`) still receive a session so they can
 * complete backend-assisted onboarding, but the token carries an explicit
 * `onboarding` scope and the refresh token uses the shorter unknown-role
 * TTL. Middleware rejects `unknown` tokens unless a route opts in.
 */
export async function issueSession(account: string) {
  const role = await deriveRole(account);
  const payload: Sep10TokenPayload & { scope?: string } =
    role === "unknown"
      ? { sub: account, role, scope: ONBOARDING_SCOPE }
      : { sub: account, role };
  const token = await issueSep10Token(payload, config);

  const refreshToken = randomBytes(32).toString("base64url");
  await prisma.refreshToken.create({
    data: {
      tokenHash: sha256Hex(refreshToken),
      account,
      expiresAt: new Date(Date.now() + refreshTokenTtlSeconds(role) * 1000),
    },
  });

  return {
    token,
    refreshToken,
    expiresIn: accessTokenTtlSeconds(),
    refreshExpiresIn: refreshTokenTtlSeconds(role),
  };
}

/**
 * Exchange a refresh token for a new session. The presented token is
 * revoked atomically (rotation), and the role is re-derived, so a validator
 * revoked since the last login is downgraded here — and a wallet that has
 * registered since its `unknown` session is upgraded to its real role with
 * the normal refresh TTL.
 */
export async function refreshSession(refreshToken: string) {
  const tokenHash = sha256Hex(refreshToken);
  const row = await prisma.refreshToken.findUnique({ where: { tokenHash } });
  if (!row) {
    throw new Error("Invalid refresh token");
  }

  const now = new Date();
  const { count } = await prisma.refreshToken.updateMany({
    where: { tokenHash, revokedAt: null, expiresAt: { gt: now } },
    data: { revokedAt: now },
  });
  if (count !== 1) {
    throw new Error("Invalid refresh token");
  }

  // issueSession re-derives the role, so the new token carries the current
  // role (and the matching refresh TTL) rather than the old one.
  return issueSession(row.account);
}

/**
 * Derive the role for a verified Stellar account from on-chain / indexer
 * state.  The priority order is admin > validator > scout > player >
 * unknown.
 */
export async function deriveRole(wallet: string): Promise<string> {
  // Check admin addresses first (from env and on-chain admin registry).
  const { adminAddress } = config;
  if (adminAddress && wallet === adminAddress) {
    return "admin";
  }

  const admin = await prisma.admin.findUnique({
    where: { address: wallet },
  });
  if (admin) {
    return "admin";
  }

  // Check validators table (must be active).
  const validator = await prisma.validator.findUnique({
    where: { wallet },
  });
  if (validator && validator.active) {
    return "validator";
  }

  // Check scouts table.
  const scout = await prisma.scout.findUnique({
    where: { wallet },
  });
  if (scout) {
    return "scout";
  }

  // Check players table.
  const player = await prisma.player.findUnique({
    where: { wallet },
  });
  if (player) {
    return "player";
  }

  return "unknown";
}

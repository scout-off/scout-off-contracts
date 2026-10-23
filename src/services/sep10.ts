import { v4 as uuidv4 } from "uuid";
import { createHash, randomBytes } from "crypto";
import jwt from "jsonwebtoken";
import { WebAuth, Keypair, Networks, TransactionBuilder } from "@stellar/stellar-sdk";
import { prisma } from "../db";

interface Sep10TokenPayload {
  sub: string;
  role: string;
}

/** Challenge validity window; also used as the WebAuth `timeout`. */
const CHALLENGE_TTL_SECONDS = 300;
/** Hard ceiling on access-token lifetime (#1606). */
const MAX_ACCESS_TOKEN_TTL_SECONDS = 15 * 60;
const DEFAULT_REFRESH_TOKEN_TTL_SECONDS = 7 * 24 * 60 * 60;

function sha256Hex(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

/**
 * Signature-independent challenge identifier: the transaction hash, which
 * stays the same when the client adds its signature. 64 hex chars, so it
 * fits the `VARCHAR(64)` key (the full base64 XDR would not).
 */
function challengeHash(challengeTx: string, networkPassphrase: string): string {
  return TransactionBuilder.fromXDR(challengeTx, networkPassphrase)
    .hash()
    .toString("hex");
}

/** Access-token lifetime in seconds: ACCESS_TOKEN_TTL_SECONDS, capped at 15 minutes. */
export function accessTokenTtlSeconds(): number {
  const configured = Number(process.env.ACCESS_TOKEN_TTL_SECONDS);
  if (!Number.isFinite(configured) || configured <= 0) {
    return MAX_ACCESS_TOKEN_TTL_SECONDS;
  }
  return Math.min(Math.floor(configured), MAX_ACCESS_TOKEN_TTL_SECONDS);
}

function refreshTokenTtlSeconds(): number {
  const configured = Number(process.env.REFRESH_TOKEN_TTL_SECONDS);
  return Number.isFinite(configured) && configured > 0
    ? Math.floor(configured)
    : DEFAULT_REFRESH_TOKEN_TTL_SECONDS;
}

export function issueSep10Token(payload: Sep10TokenPayload, secret: string) {
  return jwt.sign(payload, secret, {
    expiresIn: accessTokenTtlSeconds(),
    jwtid: uuidv4(),
  });
}

function sep10Config() {
  return {
    serverKeypair: Keypair.fromSecret(process.env.SEP10_SIGNING_SECRET!),
    homeDomain: process.env.HOME_DOMAIN!,
    webAuthDomain: process.env.WEB_AUTH_DOMAIN!,
    networkPassphrase:
      process.env.STELLAR_NETWORK === "mainnet"
        ? Networks.PUBLIC
        : Networks.TESTNET,
  };
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

  await prisma.sep10Challenge.create({
    data: {
      hash: challengeHash(challenge, networkPassphrase),
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
  const { clientAccountID } = WebAuth.readChallengeTx(
    challengeTx,
    serverAccount,
    networkPassphrase,
    homeDomain,
    webAuthDomain
  );

  if (clientAccountID !== clientAccount) {
    throw new Error("Challenge account mismatch");
  }

  // The client account must have signed the transaction. Done before the
  // challenge is consumed so an unsigned submission cannot burn it.
  WebAuth.verifyChallengeTxSigners(
    challengeTx,
    serverAccount,
    networkPassphrase,
    [clientAccount],
    homeDomain,
    webAuthDomain
  );

  // Replay protection: atomically mark the challenge consumed. Zero rows
  // updated means it is unknown, expired, or already used.
  const now = new Date();
  const { count } = await prisma.sep10Challenge.updateMany({
    where: {
      hash: challengeHash(challengeTx, networkPassphrase),
      account: clientAccount,
      consumedAt: null,
      expiresAt: { gt: now },
    },
    data: { consumedAt: now },
  });
  if (count !== 1) {
    throw new Error("Challenge has already been used");
  }

  return clientAccountID;
}

/**
 * Issue an access token plus a rotating refresh token for `account`, with
 * the role freshly derived from indexer state.
 */
export async function issueSession(account: string) {
  const role = await deriveRole(account);
  const token = issueSep10Token({ sub: account, role }, process.env.JWT_SECRET!);

  const refreshToken = randomBytes(32).toString("base64url");
  await prisma.refreshToken.create({
    data: {
      tokenHash: sha256Hex(refreshToken),
      account,
      expiresAt: new Date(Date.now() + refreshTokenTtlSeconds() * 1000),
    },
  });

  return { token, refreshToken, expiresIn: accessTokenTtlSeconds() };
}

/**
 * Exchange a refresh token for a new session. The presented token is
 * revoked atomically (rotation), and the role is re-derived, so a validator
 * revoked since the last login is downgraded here.
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

  return issueSession(row.account);
}

/**
 * Derive the role for a verified Stellar account from on-chain / indexer
 * state.  The priority order is admin > validator > scout > player >
 * unknown.
 */
export async function deriveRole(wallet: string): Promise<string> {
  // Check admin addresses first (from env and on-chain admin registry).
  const adminAddress = process.env.ADMIN_ADDRESS;
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

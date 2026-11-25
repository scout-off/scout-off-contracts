import { randomUUID } from "node:crypto";
import jwt from "jsonwebtoken";
import type { Request } from "express";

interface Sep10TokenPayload {
  sub: string;
  role: string;
}

export function issueSep10Token(payload: Sep10TokenPayload, secret: string) {
  return jwt.sign(payload, secret, {
    expiresIn: "7d",
    jwtid: randomUUID(),
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
      hash: challengeHash(tx),
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
  const token = await issueSep10Token({ sub: account, role }, config);

  const refreshToken = randomBytes(32).toString("base64url");
  await prisma.refreshToken.create({
    data: {
      tokenHash: sha256Hex(refreshToken),
      account,
      expiresAt: new Date(Date.now() + refreshTokenTtlSeconds() * 1000),
    },
  });

  return {
    token,
    refreshToken,
    expiresIn: accessTokenTtlSeconds(),
    refreshExpiresIn: refreshTokenTtlSeconds(),
  };
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

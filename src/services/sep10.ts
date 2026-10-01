import { v4 as uuidv4 } from "uuid";
import jwt from "jsonwebtoken";
import { WebAuth, Keypair, Networks, TimeBounds } from "@stellar/stellar-sdk";
import { prisma } from "../db";

interface Sep10TokenPayload {
  sub: string;
  role: string;
}

export function issueSep10Token(payload: Sep10TokenPayload, secret: string) {
  return jwt.sign(payload, secret, {
    expiresIn: "7d",
    jwtid: uuidv4(),
  });
}

/**
 * Build a SEP-10 challenge transaction for the given Stellar account.
 *
 * The challenge is a time-bounded transaction containing a manage_data
 * operation that stores the home domain.  The server signs it with its
 * signing keypair; the client must later sign it with their account key.
 */
export function buildChallengeTx(account: string): string {
  const serverKeypair = Keypair.fromSecret(
    process.env.SEP10_SIGNING_SECRET!
  );
  const homeDomain = process.env.HOME_DOMAIN!;
  const webAuthDomain = process.env.WEB_AUTH_DOMAIN!;
  const networkPassphrase =
    process.env.STELLAR_NETWORK === "mainnet"
      ? Networks.PUBLIC_NETWORK_PASSPHRASE
      : Networks.TESTNET_PASSPHRASE;

  const timeBounds = new TimeBounds(
    0,
    Math.floor(Date.now() / 1000) + 300
  );

  const challenge = WebAuth.buildChallengeTx(
    serverKeypair.secret(),
    account,
    homeDomain,
    webAuthDomain,
    networkPassphrase,
    timeBounds
  );

  return challenge;
}

/**
 * Verify a client-signed SEP-10 challenge transaction and return the
 * verified Stellar account ID.
 *
 * Throws if the challenge is invalid, expired, replayed, or signed by
 * an unauthorised signer.
 */
export async function verifyChallengeTx(
  challengeTx: string,
  clientAccount: string
): Promise<string> {
  const serverKeypair = Keypair.fromSecret(
    process.env.SEP10_SIGNING_SECRET!
  );
  const homeDomain = process.env.HOME_DOMAIN!;
  const webAuthDomain = process.env.WEB_AUTH_DOMAIN!;
  const networkPassphrase =
    process.env.STELLAR_NETWORK === "mainnet"
      ? Networks.PUBLIC_NETWORK_PASSPHRASE
      : Networks.TESTNET_PASSPHRASE;

  // readChallengeTx validates the structure, time bounds, sequence,
  // home domain, web auth domain and network passphrase.  It returns
  // the account ID that the challenge was built for.
  const verifiedAccount = WebAuth.readChallengeTx(
    challengeTx,
    serverKeypair.secret(),
    homeDomain,
    webAuthDomain,
    networkPassphrase
  );

  // verifyChallengeTxSigners checks that the client account (and any
  // additional signers) have signed the transaction.
  WebAuth.verifyChallengeTxSigners(
    challengeTx,
    serverKeypair.secret(),
    homeDomain,
    webAuthDomain,
    networkPassphrase,
    [clientAccount]
  );

  if (verifiedAccount !== clientAccount) {
    throw new Error("Challenge account mismatch");
  }

  // Replay protection: reject if this challenge has already been used.
  const existing = await prisma.revokedToken.findFirst({
    where: { jti: challengeTx },
  });
  if (existing) {
    throw new Error("Challenge has already been used");
  }

  return verifiedAccount;
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

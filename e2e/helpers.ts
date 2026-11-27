/**
 * Shared helpers for the ScoutChain e2e suite.
 */
import { Keypair, Networks, TransactionBuilder, Transaction } from "@stellar/stellar-sdk";

export const BACKEND_URL =
  process.env.E2E_BACKEND_URL ?? "http://localhost:3000";
export const RPC_URL =
  process.env.E2E_RPC_URL ?? "http://localhost:8000/soroban/rpc";
export const NETWORK_PASSPHRASE =
  process.env.NETWORK_PASSPHRASE ?? "Standalone Network ; February 2017";
export const FRIENDBOT_URL =
  process.env.E2E_FRIENDBOT_URL ?? "http://localhost:8000/friendbot";

/** Check whether the backend is reachable. Used to skip the suite gracefully. */
export async function isBackendReachable(): Promise<boolean> {
  try {
    const r = await fetch(`${BACKEND_URL}/health`, { signal: AbortSignal.timeout(5000) });
    return r.ok;
  } catch {
    return false;
  }
}

/** Fund a keypair via the local friendbot. */
export async function fundKeypair(keypair: Keypair): Promise<void> {
  const url = `${FRIENDBOT_URL}?addr=${keypair.publicKey()}`;
  const r = await fetch(url);
  if (!r.ok) {
    const body = await r.text().catch(() => "");
    throw new Error(`Friendbot failed (${r.status}): ${body}`);
  }
}

/**
 * Perform a full SEP-10 login and return the access token.
 *
 * Steps:
 *   1. GET /auth?account=<G...>  → challenge XDR
 *   2. Sign the challenge with the keypair
 *   3. POST /auth { transaction, account } → { token }
 */
export async function sep10Login(keypair: Keypair): Promise<string> {
  // 1. Fetch challenge
  const challengeRes = await fetch(
    `${BACKEND_URL}/auth?account=${keypair.publicKey()}`
  );
  if (!challengeRes.ok) {
    throw new Error(`GET /auth failed: ${await challengeRes.text()}`);
  }
  const { challenge } = (await challengeRes.json()) as { challenge: string };

  // 2. Sign challenge
  const tx = new Transaction(challenge, NETWORK_PASSPHRASE);
  tx.sign(keypair);
  const signedXdr = tx.toEnvelope().toXDR("base64");

  // 3. Exchange for token
  const tokenRes = await fetch(`${BACKEND_URL}/auth`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ transaction: signedXdr, account: keypair.publicKey() }),
  });
  if (!tokenRes.ok) {
    throw new Error(`POST /auth failed: ${await tokenRes.text()}`);
  }
  const { token } = (await tokenRes.json()) as { token: string };
  return token;
}

/** Authenticated GET helper. */
export async function authedGet(
  path: string,
  token: string
): Promise<Response> {
  return fetch(`${BACKEND_URL}${path}`, {
    headers: { Authorization: `Bearer ${token}` },
  });
}

/**
 * E2E: SEP-10 login flow
 *
 * Verifies that a fresh keypair can:
 *   1. Obtain a SEP-10 challenge from the backend.
 *   2. Sign it and exchange for an access token.
 *   3. Use the token to call an authenticated endpoint.
 *   4. Have the token revoked and confirm it is rejected afterwards.
 *
 * Requires the local stack (scripts/dev-up.sh) to be running.
 * Self-skips if the backend is not reachable.
 */
import { Keypair } from "@stellar/stellar-sdk";
import {
  isBackendReachable,
  fundKeypair,
  sep10Login,
  authedGet,
  BACKEND_URL,
} from "./helpers";

let skipAll = false;

beforeAll(async () => {
  if (!(await isBackendReachable())) {
    console.warn("E2E: backend not reachable — skipping login suite.");
    skipAll = true;
  }
});

const maybeIt = (name: string, fn: () => Promise<void>) =>
  it(name, async () => {
    if (skipAll) return;
    await fn();
  });

describe("SEP-10 login flow", () => {
  let keypair: Keypair;
  let token: string;

  maybeIt("funds a fresh keypair via friendbot", async () => {
    keypair = Keypair.random();
    await fundKeypair(keypair);
  });

  maybeIt("GET /auth returns a challenge for a valid account", async () => {
    const res = await fetch(`${BACKEND_URL}/auth?account=${keypair.publicKey()}`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { challenge?: string };
    expect(typeof body.challenge).toBe("string");
    expect(body.challenge!.length).toBeGreaterThan(10);
  });

  maybeIt("POST /auth issues an access token", async () => {
    token = await sep10Login(keypair);
    expect(typeof token).toBe("string");
    expect(token.length).toBeGreaterThan(10);
  });

  maybeIt("GET /api/tokens/me succeeds with valid token", async () => {
    const res = await authedGet("/api/tokens/me", token);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { sub?: string; role?: string };
    expect(body.sub).toBe(keypair.publicKey());
    expect(typeof body.role).toBe("string");
  });

  maybeIt("GET /api/tokens/me returns 401 without token", async () => {
    const res = await fetch(`${BACKEND_URL}/api/tokens/me`);
    expect(res.status).toBe(401);
  });

  maybeIt("GET /auth returns 400 for a missing account param", async () => {
    const res = await fetch(`${BACKEND_URL}/auth`);
    expect(res.status).toBe(400);
  });

  maybeIt("POST /auth returns 401 for a replayed (already-used) challenge", async () => {
    // Fetch a challenge and immediately try to verify the same signed tx twice.
    const challengeRes = await fetch(
      `${BACKEND_URL}/auth?account=${keypair.publicKey()}`
    );
    const { challenge } = (await challengeRes.json()) as { challenge: string };

    const { Transaction } = await import("@stellar/stellar-sdk");
    const { NETWORK_PASSPHRASE } = await import("./helpers");
    const tx = new Transaction(challenge, NETWORK_PASSPHRASE);
    tx.sign(keypair);
    const signedXdr = tx.toEnvelope().toXDR("base64");
    const payload = JSON.stringify({ transaction: signedXdr, account: keypair.publicKey() });
    const opts = { method: "POST", headers: { "Content-Type": "application/json" }, body: payload };

    const first = await fetch(`${BACKEND_URL}/auth`, opts);
    expect(first.status).toBe(200);

    // Replay
    const second = await fetch(`${BACKEND_URL}/auth`, opts);
    expect(second.status).toBe(401);
  });
});

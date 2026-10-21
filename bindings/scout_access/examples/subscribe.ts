/**
 * Reference implementation for `subscribeScout()` and `payToContact()`.
 *
 * The production scout onboarding flow consumed by ScoutChain clients lives in
 * the separate `scoutchain-backend` repo (see `../../../ai.md` and
 * `../../../README.md` — "Backend and frontend repos"). This file is the
 * pattern to port into that backend (or a frontend wallet integration) to make
 * subscription purchases and contact-unlock transactions submit real on-chain
 * state rather than stubs.
 *
 * This file is not imported anywhere in `scout-off-contracts` — it is a
 * reference implementation only. It covers the two-step scout onboarding flow
 * documented in the scout_access README:
 *
 *   Step 1 — subscribe(scout, tier)
 *     Purchases a Basic, Pro, or Elite subscription. The XLM fee is
 *     transferred from the scout's wallet to the contract atomically.
 *     Auth: the scout's wallet must sign.
 *     Errors:
 *       SubscriptionDowngradeNotAllowed (12) — downgrade while active
 *       UpgradeTooSoon (17)                  — < 1 hour since last subscribe
 *       InvalidInput (15)                    — bad tier value
 *       ContractPaused (code varies)         — circuit breaker active
 *
 *   Step 2 — pay_to_contact(scout, player_id)
 *     Unlocks a player's contact details. Scout must have an active
 *     subscription. Basic-tier scouts cannot call this.
 *     Auth: the scout's wallet must sign.
 *     Errors:
 *       ScoutNotSubscribed (6)           — no active subscription
 *       SubscriptionExpired (varies)     — subscription has lapsed
 *       InsufficientTier (varies)        — Basic scout attempting contact
 *       ProContactLimitReached (20)      — Pro scout hit monthly cap
 *       AlreadyContacted (varies)        — scout already contacted this player
 *       PlayerNotFound (varies)          — invalid player_id
 *
 * See `contracts/scout_access/src/lib.rs` and `docs/CONTRACT_REFERENCE.md`
 * for the on-chain contract this wraps.
 *
 * Note on generated types: the exact shape of `AssembledTransaction<T>.result`
 * for a fallible contract method depends on the pinned `stellar-cli` version
 * used by `scripts/generate-bindings.sh` (currently 25.2.0). This file
 * defensively checks for both shapes — verify against your generated
 * `src/index.ts` and adjust if a newer codegen version changes that convention.
 */
import { Client as ScoutAccessClient, networks } from "@scoutchain/bindings-scout-access";
import type { SignTransaction } from "@stellar/stellar-sdk/contract";

// ---------------------------------------------------------------------------
// subscribe()
// ---------------------------------------------------------------------------

export interface SubscribeParams {
  /** Address of the scout purchasing the subscription. Must sign. */
  scout: string;
  /**
   * Subscription tier to purchase.
   * - Basic  — browse verified players (Level 1+); cannot call pay_to_contact
   * - Pro    — browse all levels + contact up to `pro_contact_limit` players/month
   * - Elite  — unlimited contacts + can log trial offers
   */
  tier: "Basic" | "Pro" | "Elite";
  network: keyof typeof networks;
  rpcUrl: string;
  /** Public key of the scout's wallet. */
  publicKey: string;
  /** Wallet signing callback (e.g. Freighter's `signTransaction`). */
  signTransaction: SignTransaction;
}

export interface SubscribeResult {
  /** Soroban transaction hash (64 lowercase hex chars). */
  transactionId: string;
}

// ---------------------------------------------------------------------------
// payToContact()
// ---------------------------------------------------------------------------

export interface PayToContactParams {
  /** Address of the scout with an active Pro or Elite subscription. Must sign. */
  scout: string;
  /** Numeric player identifier to unlock contact details for. */
  playerId: bigint;
  network: keyof typeof networks;
  rpcUrl: string;
  /** Public key of the scout's wallet. */
  publicKey: string;
  /** Wallet signing callback (e.g. Freighter's `signTransaction`). */
  signTransaction: SignTransaction;
}

export interface PayToContactResult {
  /** Soroban transaction hash (64 lowercase hex chars). */
  transactionId: string;
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

interface ResultLike<T, E> {
  isErr(): boolean;
  unwrap(): T;
  unwrapErr(): E;
}

function isResultLike(value: unknown): value is ResultLike<unknown, unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as ResultLike<unknown, unknown>).isErr === "function"
  );
}

function assertSuccess(
  sent: {
    result: unknown;
    sendTransactionResponse?: { hash?: string };
    getTransactionResponse?: { status?: string };
  },
  context: string,
): string {
  if (isResultLike(sent.result) && sent.result.isErr()) {
    throw new Error(
      `${context}: contract returned error: ${String(sent.result.unwrapErr())}`,
    );
  }

  const transactionId = sent.sendTransactionResponse?.hash;
  if (sent.getTransactionResponse?.status !== "SUCCESS" || !transactionId) {
    throw new Error(
      `${context}: transaction did not confirm as SUCCESS ` +
        `(status: ${sent.getTransactionResponse?.status ?? "unknown"})`,
    );
  }

  return transactionId;
}

// ---------------------------------------------------------------------------
// Exported functions
// ---------------------------------------------------------------------------

/**
 * Purchase a scout subscription at the given tier.
 *
 * The XLM fee is read from `get_fee_config()` before submitting — the exact
 * amount is determined on-chain; ensure the scout's wallet has sufficient
 * balance. Call `client.get_fee_config()` beforehand if you need to display
 * the fee to the user before prompting them to sign.
 */
export async function subscribeScout(params: SubscribeParams): Promise<SubscribeResult> {
  const { scout, tier, network, rpcUrl, publicKey, signTransaction } = params;

  const client = new ScoutAccessClient({
    ...networks[network],
    rpcUrl,
    publicKey,
    signTransaction,
  });

  let sent;
  try {
    const assembled = await client.subscribe({
      scout,
      tier: { [tier]: {} } as Parameters<typeof client.subscribe>[0]["tier"],
    });
    sent = await assembled.signAndSend();
  } catch (err) {
    throw new Error(
      `subscribeScout: subscribe Soroban transaction failed for scout ${scout} (tier: ${tier}): ` +
        `${err instanceof Error ? err.message : String(err)}`,
      { cause: err instanceof Error ? err : undefined },
    );
  }

  const transactionId = assertSuccess(sent, `subscribeScout(${scout}, ${tier})`);
  return { transactionId };
}

/**
 * Pay the contact fee to unlock a player's details.
 *
 * The scout must hold an active Pro or Elite subscription. Basic-tier scouts
 * will receive an on-chain error. Already-contacted players are rejected
 * on-chain as well — call `client.has_contacted(scout, player_id)` first if
 * you want to guard against a redundant fee payment before signing.
 */
export async function payToContact(params: PayToContactParams): Promise<PayToContactResult> {
  const { scout, playerId, network, rpcUrl, publicKey, signTransaction } = params;

  const client = new ScoutAccessClient({
    ...networks[network],
    rpcUrl,
    publicKey,
    signTransaction,
  });

  let sent;
  try {
    const assembled = await client.pay_to_contact({
      scout,
      player_id: playerId,
    });
    sent = await assembled.signAndSend();
  } catch (err) {
    throw new Error(
      `payToContact: pay_to_contact Soroban transaction failed for scout ${scout}, ` +
        `player ${playerId}: ${err instanceof Error ? err.message : String(err)}`,
      { cause: err instanceof Error ? err : undefined },
    );
  }

  const transactionId = assertSuccess(
    sent,
    `payToContact(scout: ${scout}, player: ${playerId})`,
  );
  return { transactionId };
}

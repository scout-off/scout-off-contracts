/**
 * Integration test for the `subscribeScout()` and `payToContact()` reference
 * implementations.
 *
 * Requires a live Soroban RPC endpoint, a funded scout wallet with enough XLM
 * to cover the subscription fee, and a pre-registered player to contact.
 * Standing up the full `initialize` + funded-signer + XLM-transfer fixture
 * is out of scope for this reference example. Port this test alongside
 * `subscribeScout()` / `payToContact()` into `scoutchain-backend`, where that
 * fixture setup already exists.
 *
 * Skips automatically unless all of the following are set:
 *   INTEGRATION_RPC_URL    e.g. https://soroban-testnet.stellar.org
 *   INTEGRATION_NETWORK    "testnet" | "mainnet" (must match the `networks` key
 *                          exported by @scoutchain/bindings-scout-access)
 *   INTEGRATION_SECRET     secret key (S...) of a funded scout wallet
 *   INTEGRATION_TIER       "Basic" | "Pro" | "Elite" — tier to subscribe at
 *   INTEGRATION_PLAYER_ID  numeric player_id of an existing active player
 *                          (only used for the pay_to_contact step; requires
 *                          a Pro or Elite tier)
 *
 * Run against testnet, e.g.:
 *   INTEGRATION_RPC_URL=https://soroban-testnet.stellar.org \
 *   INTEGRATION_NETWORK=testnet \
 *   INTEGRATION_SECRET=S... \
 *   INTEGRATION_TIER=Pro \
 *   INTEGRATION_PLAYER_ID=1 \
 *   npx vitest run examples/subscribe.integration.test.ts
 *
 * Note on fees: the XLM fee is set by the deployed contract's FeeConfig.
 * On testnet, fund the scout wallet via Friendbot before running:
 *   curl "https://friendbot.stellar.org?addr=<PUBLIC_KEY>"
 */
import { describe, it, expect } from "vitest";
import { Keypair, TransactionBuilder } from "@stellar/stellar-base";
import {
  Client as ScoutAccessClient,
  networks,
} from "@scoutchain/bindings-scout-access";
import { subscribeScout, payToContact } from "./subscribe";

const {
  INTEGRATION_RPC_URL,
  INTEGRATION_NETWORK,
  INTEGRATION_SECRET,
  INTEGRATION_TIER,
  INTEGRATION_PLAYER_ID,
} = process.env;

const hasLiveConfig = Boolean(
  INTEGRATION_RPC_URL &&
    INTEGRATION_NETWORK &&
    INTEGRATION_SECRET &&
    INTEGRATION_TIER &&
    INTEGRATION_PLAYER_ID,
);

describe.skipIf(!hasLiveConfig)("subscribeScout + payToContact (live contract)", () => {
  const network = INTEGRATION_NETWORK as keyof typeof networks;
  const rpcUrl = INTEGRATION_RPC_URL as string;
  const tier = INTEGRATION_TIER as "Basic" | "Pro" | "Elite";
  const keypair = Keypair.fromSecret(INTEGRATION_SECRET as string);
  const playerId = BigInt(INTEGRATION_PLAYER_ID as string);

  const signTransaction: Parameters<typeof subscribeScout>[0]["signTransaction"] = async (xdr) => {
    // Minimal local signer standing in for a browser wallet's
    // signTransaction callback.
    const tx = TransactionBuilder.fromXDR(xdr, networks[network].networkPassphrase);
    tx.sign(keypair);
    return { signedTxXdr: tx.toXDR() };
  };

  it("submits subscribe on-chain and the subscription is readable back from the contract", async () => {
    const result = await subscribeScout({
      scout: keypair.publicKey(),
      tier,
      network,
      rpcUrl,
      publicKey: keypair.publicKey(),
      signTransaction,
    });

    // Soroban transaction hashes are 64 lowercase hex characters.
    expect(result.transactionId).toMatch(/^[0-9a-f]{64}$/i);

    // Re-query the contract directly (no signer needed for a read call) to
    // confirm the subscription actually landed on-chain, not just that the
    // transaction was accepted.
    const readClient = new ScoutAccessClient({
      ...networks[network],
      rpcUrl,
    });

    const subAssembled = await readClient.get_subscription({
      scout: keypair.publicKey(),
    });

    const subscription =
      typeof (subAssembled.result as { unwrap?: () => unknown })?.unwrap === "function"
        ? (
            subAssembled.result as {
              unwrap: () => {
                tier: unknown;
                expires_at: bigint;
                subscribed_at: bigint;
              };
            }
          ).unwrap()
        : (subAssembled.result as {
            tier: unknown;
            expires_at: bigint;
            subscribed_at: bigint;
          });

    // Tier should match what we subscribed at.
    expect(JSON.stringify(subscription.tier)).toContain(tier);

    // expires_at is in the future (after the current Unix timestamp).
    const nowSeconds = BigInt(Math.floor(Date.now() / 1000));
    expect(subscription.expires_at).toBeGreaterThan(nowSeconds);
    // subscribed_at is in the recent past (within the last 5 minutes).
    expect(subscription.subscribed_at).toBeGreaterThan(nowSeconds - BigInt(300));
  });

  it("submits pay_to_contact on-chain and has_contacted returns true afterwards (Pro/Elite only)", async () => {
    // Basic scouts cannot call pay_to_contact — skip this assertion for Basic.
    if (tier === "Basic") {
      return;
    }

    const result = await payToContact({
      scout: keypair.publicKey(),
      playerId,
      network,
      rpcUrl,
      publicKey: keypair.publicKey(),
      signTransaction,
    });

    expect(result.transactionId).toMatch(/^[0-9a-f]{64}$/i);

    // Re-query the contract directly to confirm the contact record is on-chain.
    const readClient = new ScoutAccessClient({
      ...networks[network],
      rpcUrl,
    });

    const hasContactedAssembled = await readClient.has_contacted({
      scout: keypair.publicKey(),
      player_id: playerId,
    });

    expect(hasContactedAssembled.result).toBe(true);
  });

  it("get_subscription returns a valid Subscription shape with the correct tier", async () => {
    const readClient = new ScoutAccessClient({
      ...networks[network],
      rpcUrl,
    });

    const assembled = await readClient.get_subscription({ scout: keypair.publicKey() });

    const sub =
      typeof (assembled.result as { unwrap?: () => unknown })?.unwrap === "function"
        ? (assembled.result as { unwrap: () => Record<string, unknown> }).unwrap()
        : (assembled.result as Record<string, unknown>);

    // The tier object is a tagged-enum from Soroban codegen (e.g. { Basic: {} }).
    expect(sub).toHaveProperty("tier");
    expect(sub).toHaveProperty("expires_at");
    expect(sub).toHaveProperty("subscribed_at");
    expect(JSON.stringify(sub.tier)).toContain(tier);
  });
});

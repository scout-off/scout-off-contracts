/**
 * Integration test for the `updateProfile()` reference implementation.
 *
 * Requires a live Soroban RPC endpoint and a pre-registered player — standing
 * up a full `register_player` + funded-signer flow is out of scope for this
 * reference example. Port this test alongside `updateProfile()` into
 * `scoutchain-backend`, where that fixture setup already exists.
 *
 * Skips automatically unless all of the following are set:
 *   INTEGRATION_RPC_URL    e.g. https://soroban-testnet.stellar.org
 *   INTEGRATION_NETWORK    "testnet" | "mainnet" (must match the `networks` key
 *                          exported by @scoutchain/bindings-registration)
 *   INTEGRATION_SECRET     secret key (S...) of an already-registered player's wallet
 *   INTEGRATION_PLAYER_ID  that wallet's on-chain player_id
 *
 * Run against testnet, e.g.:
 *   INTEGRATION_RPC_URL=https://soroban-testnet.stellar.org \
 *   INTEGRATION_NETWORK=testnet \
 *   INTEGRATION_SECRET=S... \
 *   INTEGRATION_PLAYER_ID=1 \
 *   npx vitest run examples/updateProfile.integration.test.ts
 */
import { describe, it, expect } from "vitest";
import { Keypair, TransactionBuilder } from "@stellar/stellar-base";
import { Client as RegistrationClient, networks } from "@scoutchain/bindings-registration";
import { updateProfile } from "./updateProfile";

const { INTEGRATION_RPC_URL, INTEGRATION_NETWORK, INTEGRATION_SECRET, INTEGRATION_PLAYER_ID } =
  process.env;

const hasLiveConfig = Boolean(
  INTEGRATION_RPC_URL && INTEGRATION_NETWORK && INTEGRATION_SECRET && INTEGRATION_PLAYER_ID,
);

describe.skipIf(!hasLiveConfig)("updateProfile (live contract)", () => {
  it("submits update_profile on-chain and the change is readable back from the contract", async () => {
    const network = INTEGRATION_NETWORK as keyof typeof networks;
    const keypair = Keypair.fromSecret(INTEGRATION_SECRET as string);
    const playerId = BigInt(INTEGRATION_PLAYER_ID as string);
    const metadataUri = `QmIntegrationTest${Date.now()}`;

    const result = await updateProfile({
      playerId,
      metadataUri,
      network,
      rpcUrl: INTEGRATION_RPC_URL as string,
      publicKey: keypair.publicKey(),
      signTransaction: async (xdr) => {
        // Minimal local signer standing in for a browser wallet's
        // signTransaction callback.
        const tx = TransactionBuilder.fromXDR(xdr, networks[network].networkPassphrase);
        tx.sign(keypair);
        return { signedTxXdr: tx.toXDR() };
      },
    });

    expect(result.metadataUri).toBe(metadataUri);
    // Soroban transaction hashes are 64 lowercase hex characters.
    expect(result.transactionId).toMatch(/^[0-9a-f]{64}$/i);

    // Re-query the contract directly (no signer needed for a read call) to
    // confirm the update actually landed on-chain, not just that the
    // transaction was accepted.
    const readClient = new RegistrationClient({
      ...networks[network],
      rpcUrl: INTEGRATION_RPC_URL as string,
    });
    const assembled = await readClient.get_player({ player_id: playerId });
    const player =
      typeof (assembled.result as { unwrap?: () => unknown })?.unwrap === "function"
        ? (assembled.result as { unwrap: () => { ipfs_hashes: string[] } }).unwrap()
        : (assembled.result as { ipfs_hashes: string[] });

    expect(player.ipfs_hashes).toContain(metadataUri);
  });
});

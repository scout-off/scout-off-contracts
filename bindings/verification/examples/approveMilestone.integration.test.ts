/**
 * Integration test for the `approveMilestone()` reference implementation.
 *
 * Requires a live Soroban RPC endpoint, a pre-registered active validator,
 * and a pre-registered player — standing up the full `register_validator` +
 * `register_player` + funded-signer flow is out of scope for this reference
 * example. Port this test alongside `approveMilestone()` into
 * `scoutchain-backend`, where that fixture setup already exists.
 *
 * Skips automatically unless all of the following are set:
 *   INTEGRATION_RPC_URL       e.g. https://soroban-testnet.stellar.org
 *   INTEGRATION_NETWORK       "testnet" | "mainnet" (must match the `networks`
 *                             key exported by @scoutchain/bindings-verification)
 *   INTEGRATION_SECRET        secret key (S...) of a registered, active validator
 *   INTEGRATION_PLAYER_ID     numeric player_id of an existing, active player
 *   INTEGRATION_EVIDENCE_HASH IPFS/Arweave CID of the evidence document, already
 *                             uploaded. Must be unique — the contract rejects
 *                             DuplicateEvidence. Use a fresh CID per test run,
 *                             e.g. append a timestamp to your test fixture.
 *
 * Run against testnet, e.g.:
 *   INTEGRATION_RPC_URL=https://soroban-testnet.stellar.org \
 *   INTEGRATION_NETWORK=testnet \
 *   INTEGRATION_SECRET=S... \
 *   INTEGRATION_PLAYER_ID=1 \
 *   INTEGRATION_EVIDENCE_HASH=QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG \
 *   npx vitest run examples/approveMilestone.integration.test.ts
 */
import { describe, it, expect } from "vitest";
import { Keypair, TransactionBuilder } from "@stellar/stellar-base";
import {
  Client as VerificationClient,
  networks,
} from "@scoutchain/bindings-verification";
import { approveMilestone } from "./approveMilestone";

const {
  INTEGRATION_RPC_URL,
  INTEGRATION_NETWORK,
  INTEGRATION_SECRET,
  INTEGRATION_PLAYER_ID,
  INTEGRATION_EVIDENCE_HASH,
} = process.env;

const hasLiveConfig = Boolean(
  INTEGRATION_RPC_URL &&
    INTEGRATION_NETWORK &&
    INTEGRATION_SECRET &&
    INTEGRATION_PLAYER_ID &&
    INTEGRATION_EVIDENCE_HASH,
);

describe.skipIf(!hasLiveConfig)("approveMilestone (live contract)", () => {
  it("submits approve_milestone on-chain and the milestone is readable back from the contract", async () => {
    const network = INTEGRATION_NETWORK as keyof typeof networks;
    const keypair = Keypair.fromSecret(INTEGRATION_SECRET as string);
    const playerId = BigInt(INTEGRATION_PLAYER_ID as string);
    const evidenceHash = INTEGRATION_EVIDENCE_HASH as string;
    const description = `Integration test milestone — ${new Date().toISOString()}`;

    const result = await approveMilestone({
      validatorWallet: keypair.publicKey(),
      playerId,
      description,
      evidenceHash,
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

    // Soroban transaction hashes are 64 lowercase hex characters.
    expect(result.transactionId).toMatch(/^[0-9a-f]{64}$/i);
    // milestoneIndex is 1-indexed and must be a positive integer.
    expect(result.milestoneIndex).toBeGreaterThan(0);

    // Re-query the contract directly (no signer needed for a read call) to
    // confirm the milestone actually landed on-chain, not just that the
    // transaction was accepted.
    const readClient = new VerificationClient({
      ...networks[network],
      rpcUrl: INTEGRATION_RPC_URL as string,
    });

    const milestoneAssembled = await readClient.get_milestone({
      player_id: playerId,
      index: result.milestoneIndex,
    });

    const milestone =
      typeof (milestoneAssembled.result as { unwrap?: () => unknown })?.unwrap === "function"
        ? (
            milestoneAssembled.result as {
              unwrap: () => {
                description: string;
                evidence_hash: string;
                validator: string;
              };
            }
          ).unwrap()
        : (milestoneAssembled.result as {
            description: string;
            evidence_hash: string;
            validator: string;
          });

    expect(milestone.description).toBe(description);
    expect(milestone.evidence_hash).toBe(evidenceHash);
    expect(String(milestone.validator)).toBe(keypair.publicKey());

    // The count should now be at least the index we just received.
    const countAssembled = await readClient.get_milestone_count({ player_id: playerId });
    expect(Number(countAssembled.result)).toBeGreaterThanOrEqual(result.milestoneIndex);
  });
});

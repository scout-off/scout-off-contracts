/**
 * Integration test for the `getPlayerHistory()` reference implementation.
 *
 * All queries in this test are read-only — no signer or funded wallet is
 * required. You only need a live Soroban RPC endpoint and a player whose
 * level has been advanced at least once by the verification or scout_access
 * contract on that network.
 *
 * Skips automatically unless all of the following are set:
 *   INTEGRATION_RPC_URL    e.g. https://soroban-testnet.stellar.org
 *   INTEGRATION_NETWORK    "testnet" | "mainnet" (must match the `networks` key
 *                          exported by @scoutchain/bindings-progress)
 *   INTEGRATION_PLAYER_ID  numeric player_id of a player with at least one
 *                          advance_level entry on-chain
 *
 * Run against testnet, e.g.:
 *   INTEGRATION_RPC_URL=https://soroban-testnet.stellar.org \
 *   INTEGRATION_NETWORK=testnet \
 *   INTEGRATION_PLAYER_ID=1 \
 *   npx vitest run examples/getPlayerHistory.integration.test.ts
 */
import { describe, it, expect } from "vitest";
import { Client as ProgressClient, networks } from "@scoutchain/bindings-progress";
import { getPlayerHistory } from "./getPlayerHistory";

const { INTEGRATION_RPC_URL, INTEGRATION_NETWORK, INTEGRATION_PLAYER_ID } = process.env;

const hasLiveConfig = Boolean(
  INTEGRATION_RPC_URL && INTEGRATION_NETWORK && INTEGRATION_PLAYER_ID,
);

describe.skipIf(!hasLiveConfig)("getPlayerHistory (live contract)", () => {
  const network = INTEGRATION_NETWORK as keyof typeof networks;
  const playerId = BigInt(INTEGRATION_PLAYER_ID as string);
  const rpcUrl = INTEGRATION_RPC_URL as string;

  it("returns a known ProgressLevel string for the player", async () => {
    const result = await getPlayerHistory({ playerId, network, rpcUrl });

    const validLevels = ["Unverified", "VerifiedIdentity", "PerformanceMilestones", "EliteTier"];
    expect(validLevels).toContain(result.currentLevel);
  });

  it("returns full history as a non-empty array with valid ProgressEntry shape", async () => {
    const result = await getPlayerHistory({ playerId, network, rpcUrl });

    // A player with at least one advance_level call must have history.
    expect(result.history.length).toBeGreaterThan(0);

    for (const entry of result.history) {
      // player_id matches
      expect(entry.player_id).toBe(playerId);

      // Both level fields are valid ProgressLevel strings
      const validLevels = ["Unverified", "VerifiedIdentity", "PerformanceMilestones", "EliteTier"];
      expect(validLevels).toContain(String(entry.old_level));
      expect(validLevels).toContain(String(entry.new_level));

      // updated_at is a plausible Unix timestamp (after 2024-01-01)
      expect(Number(entry.updated_at)).toBeGreaterThan(1_704_067_200);

      // updated_by is a valid Stellar account address (G...)
      expect(String(entry.updated_by)).toMatch(/^G[A-Z2-7]{55}$/);

      // ledger_sequence is a positive integer
      expect(entry.ledger_sequence).toBeGreaterThan(0);
    }
  });

  it("history entries are in chronological order (oldest first)", async () => {
    const result = await getPlayerHistory({ playerId, network, rpcUrl });

    if (result.history.length < 2) {
      // Can't test ordering with fewer than two entries; just pass.
      return;
    }

    for (let i = 1; i < result.history.length; i++) {
      expect(Number(result.history[i].updated_at)).toBeGreaterThanOrEqual(
        Number(result.history[i - 1].updated_at),
      );
    }
  });

  it("sinceTimestamp filter returns a subset of the full history", async () => {
    const full = await getPlayerHistory({ playerId, network, rpcUrl });

    if (full.history.length < 2) {
      // Not enough entries to test the filter; just pass.
      return;
    }

    // Use the timestamp of the second entry as the cutoff so we expect to drop
    // at least the first entry.
    const cutoff = full.history[1].updated_at;
    const filtered = await getPlayerHistory({ playerId, network, rpcUrl, sinceTimestamp: cutoff });

    expect(filtered.history.length).toBeLessThanOrEqual(full.history.length);
    for (const entry of filtered.history) {
      expect(Number(entry.updated_at)).toBeGreaterThanOrEqual(Number(cutoff));
    }
  });

  it("paginated results are consistent with the full history", async () => {
    const full = await getPlayerHistory({ playerId, network, rpcUrl });

    // Fetch the first page (up to 5 entries) and verify it matches the start
    // of the full history.
    const pageSize = Math.min(5, full.history.length);
    const paged = await getPlayerHistory({
      playerId,
      network,
      rpcUrl,
      page: { offset: 0, limit: pageSize },
    });

    expect(paged.history.length).toBe(pageSize);
    for (let i = 0; i < pageSize; i++) {
      expect(paged.history[i].updated_at).toEqual(full.history[i].updated_at);
      expect(String(paged.history[i].new_level)).toEqual(String(full.history[i].new_level));
    }
  });

  it("get_history_count matches the number of entries returned by get_progress_history", async () => {
    // Re-query the contract directly (no wrapper) to cross-check the count.
    const client = new ProgressClient({
      ...networks[network],
      rpcUrl,
    });

    const countAssembled = await client.get_history_count({ player_id: playerId });
    const count = Number(countAssembled.result);

    const result = await getPlayerHistory({ playerId, network, rpcUrl });
    expect(result.history.length).toBe(count);
  });

  it("returns Unverified and empty history for a player_id that does not exist", async () => {
    // Use a very large player_id that is extremely unlikely to exist on-chain.
    const nonExistentId = BigInt("999999999999");

    const result = await getPlayerHistory({
      playerId: nonExistentId,
      network,
      rpcUrl,
    });

    // get_level returns Unverified rather than throwing for unknown players.
    expect(result.currentLevel).toBe("Unverified");
    // get_progress_history returns [] for unknown players.
    expect(result.history).toEqual([]);
  });
});

/**
 * Reference implementation for `getPlayerHistory()`.
 *
 * Reads a player's current progress level and their full level-advancement
 * history from the ScoutChain progress contract via the generated
 * `@scoutchain/bindings-progress` client.
 *
 * The progress contract's write path (`advance_level`) is cross-contract only
 * — it is called exclusively by the verification contract when a validator
 * approves a milestone, and by scout_access when a player confirms a trial
 * offer. There is no way to call it directly from TypeScript in production.
 * This example therefore focuses on the read path, which is what frontend and
 * backend integrators will reach for most often: "what level is this player,
 * and how did they get here?"
 *
 * Covered functions:
 *   get_level(player_id)            → ProgressLevel (Unverified for unknown IDs)
 *   get_progress_history(player_id) → Vec<ProgressEntry> (O(1) single-key read)
 *   get_history_since(player_id, since_timestamp) → Vec<ProgressEntry>
 *   get_progress_history_page(player_id, offset, limit) → Vec<ProgressEntry>
 *
 * See `contracts/progress/src/lib.rs` and `docs/CONTRACT_REFERENCE.md` for the
 * on-chain contract this wraps.
 *
 * Note on generated types: the exact shape of `AssembledTransaction<T>.result`
 * depends on the pinned `stellar-cli` version used by
 * `scripts/generate-bindings.sh` (currently 25.2.0). This file defensively
 * handles both the `Ok<T>|Err<E>` union and the auto-thrown codegen shape —
 * verify against your generated `src/index.ts` and adjust if needed.
 */
import { Client as ProgressClient, networks } from "@scoutchain/bindings-progress";

export interface GetPlayerHistoryParams {
  playerId: bigint;
  network: keyof typeof networks;
  rpcUrl: string;
  /**
   * Optional: only return history entries at or after this Unix timestamp
   * (seconds). When omitted, all history is returned.
   */
  sinceTimestamp?: bigint;
  /**
   * Optional: paginate results. When provided, `offset` and `limit` are
   * forwarded to `get_progress_history_page`. `limit` is clamped to 1–50
   * on-chain. Ignored when `sinceTimestamp` is also provided.
   */
  page?: { offset: number; limit: number };
}

export interface ProgressEntry {
  player_id: bigint;
  old_level: string;
  new_level: string;
  updated_by: string;
  /** Unix seconds */
  updated_at: bigint;
  milestone_ref: number;
  ledger_sequence: number;
}

export interface GetPlayerHistoryResult {
  /** Current level: Unverified | VerifiedIdentity | PerformanceMilestones | EliteTier */
  currentLevel: string;
  /** History entries, oldest-first */
  history: ProgressEntry[];
}

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

/** Unwrap an AssembledTransaction result, handling both codegen shapes. */
function unwrapResult<T>(result: unknown, context: string): T {
  if (isResultLike(result)) {
    if (result.isErr()) {
      throw new Error(`${context}: contract returned error: ${String(result.unwrapErr())}`);
    }
    return result.unwrap() as T;
  }
  return result as T;
}

export async function getPlayerHistory(
  params: GetPlayerHistoryParams,
): Promise<GetPlayerHistoryResult> {
  const { playerId, network, rpcUrl, sinceTimestamp, page } = params;

  // Read-only queries need no publicKey or signTransaction.
  const client = new ProgressClient({
    ...networks[network],
    rpcUrl,
  });

  // --- current level ---
  let currentLevel: string;
  try {
    const assembled = await client.get_level({ player_id: playerId });
    // get_level returns ProgressLevel directly (not a Result), so no isErr check.
    currentLevel = String(assembled.result);
  } catch (err) {
    throw new Error(
      `getPlayerHistory: get_level failed for player ${playerId}: ` +
        `${err instanceof Error ? err.message : String(err)}`,
      { cause: err instanceof Error ? err : undefined },
    );
  }

  // --- history ---
  let history: ProgressEntry[];

  if (sinceTimestamp !== undefined) {
    // Incremental: only entries at or after the caller's last sync point.
    try {
      const assembled = await client.get_history_since({
        player_id: playerId,
        since_timestamp: sinceTimestamp,
      });
      history = unwrapResult<ProgressEntry[]>(assembled.result, `get_history_since(${playerId})`);
    } catch (err) {
      throw new Error(
        `getPlayerHistory: get_history_since failed for player ${playerId}: ` +
          `${err instanceof Error ? err.message : String(err)}`,
        { cause: err instanceof Error ? err : undefined },
      );
    }
  } else if (page !== undefined) {
    // Paginated: useful when the full history is long and the caller only needs
    // a single page. limit is clamped to 1–50 on-chain.
    try {
      const assembled = await client.get_progress_history_page({
        player_id: playerId,
        offset: page.offset,
        limit: page.limit,
      });
      history = unwrapResult<ProgressEntry[]>(
        assembled.result,
        `get_progress_history_page(${playerId})`,
      );
    } catch (err) {
      throw new Error(
        `getPlayerHistory: get_progress_history_page failed for player ${playerId}: ` +
          `${err instanceof Error ? err.message : String(err)}`,
        { cause: err instanceof Error ? err : undefined },
      );
    }
  } else {
    // Full history — O(1) single-key read regardless of entry count.
    try {
      const assembled = await client.get_progress_history({ player_id: playerId });
      history = unwrapResult<ProgressEntry[]>(
        assembled.result,
        `get_progress_history(${playerId})`,
      );
    } catch (err) {
      throw new Error(
        `getPlayerHistory: get_progress_history failed for player ${playerId}: ` +
          `${err instanceof Error ? err.message : String(err)}`,
        { cause: err instanceof Error ? err : undefined },
      );
    }
  }

  return { currentLevel, history };
}

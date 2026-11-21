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
 *   get_level(player_id)                    → ProgressLevel (Unverified for unknown IDs)
 *   get_progress_history(player_id)         → Vec<ProgressEntry> (DEPRECATED: unbounded cost)
 *   get_history_since_v2(player_id, since_timestamp, limit) → HistorySince { entries, truncated }
 *   get_history_since(player_id, since_timestamp, limit) → Vec<ProgressEntry>
 *       (SUPERSEDED by get_history_since_v2: silently truncates at `limit` or the
 *        10-page scan cap, so entries can be dropped without notice. Prefer v2.)
 *   get_progress_history_page(player_id, offset, limit) → Vec<ProgressEntry>
 *   get_history_page_with_cursor(player_id, cursor_snapshot?, cursor_next_index?, limit) → Vec<ProgressEntry>
 *
 * When using `get_history_since_v2`, check the returned `truncated` flag: when it
 * is true, more matching entries may exist beyond `limit` (or beyond the scan
 * cap). Page further with `get_history_page_with_cursor`, advancing the cursor
 * from the last entry you received, until `truncated` is false.
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
   * (seconds). When omitted, all history is returned (DEPRECATED: unbounded cost).
   */
  sinceTimestamp?: bigint;
  /**
   * Optional: maximum number of entries to return for `get_history_since_v2`.
   * Clamped to 1–50 on-chain. Only used when `sinceTimestamp` is provided.
   */
  sinceLimit?: number;
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
  /**
   * True when more matching entries may exist beyond `sinceLimit` (or beyond the
   * on-chain scan cap). Page further with `get_history_page_with_cursor` until
   * this is false. Always false for the non-`sinceTimestamp` branches.
   */
  truncated: boolean;
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
  let truncated = false;

  if (sinceTimestamp !== undefined) {
    // Incremental: only entries at or after the caller's last sync point.
    // Uses get_history_since_v2 (preferred over get_history_since, which
    // silently truncates): returns up to `sinceLimit` (default 50) matching
    // entries plus a `truncated` flag telling you whether more may exist.
    try {
      const assembled = await client.get_history_since_v2({
        player_id: playerId,
        since_timestamp: sinceTimestamp,
        limit: sinceLimit ?? 50,
      });
      const result = unwrapResult<{ entries: ProgressEntry[]; truncated: boolean }>(
        assembled.result,
        `get_history_since_v2(${playerId})`,
      );
      history = result.entries;
      truncated = result.truncated;
    } catch (err) {
      throw new Error(
        `getPlayerHistory: get_history_since_v2 failed for player ${playerId}: ` +
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

  return { currentLevel, history, truncated };
}

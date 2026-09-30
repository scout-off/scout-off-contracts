/**
 * Reference implementation for `updateProfile()`.
 *
 * The production `updateProfile()` consumed by ScoutChain clients lives in the
 * separate `scoutchain-backend` repo (see `../../../ai.md` and
 * `../../../README.md` — "Backend and frontend repos"). As of writing, that
 * function returns `stub-update-txid-${playerId.slice(0, 8)}` without ever
 * touching the chain, so on-chain profile updates are silently dropped after
 * IPFS pinning.
 *
 * This file is not imported anywhere in `scout-off-contracts` — it is the
 * pattern to port into `scoutchain-backend` to close that gap. It builds and
 * submits a real `update_profile(player_id, ipfs_hashes)` Soroban transaction
 * via the generated `@scoutchain/bindings-registration` client and returns
 * the transaction hash Soroban RPC actually assigned to it
 * (`sendTransactionResponse.hash`), not a synthetic string.
 *
 * See `contracts/registration/src/lib.rs::update_profile` and
 * `docs/CONTRACT_REFERENCE.md` for the on-chain contract this wraps:
 *
 *   update_profile(player_id: u64, ipfs_hashes: Vec<String>) -> Result<(), ScoutChainError>
 *   Auth: the player's wallet must sign.
 *   Errors: PlayerNotFound, InvalidInput (empty or >10 hashes), ContractPaused
 *
 * Note on generated types: the exact shape of `AssembledTransaction<T>.result`
 * for a fallible contract method (`Ok<T> | Err<E>` vs. an auto-thrown error)
 * depends on the pinned `stellar-cli` version used by
 * `scripts/generate-bindings.sh` (currently 25.2.0). This file defensively
 * checks for both shapes — verify against your generated `src/index.ts` and
 * adjust if a newer codegen version changes that convention.
 */
import { Client as RegistrationClient, networks } from "@scoutchain/bindings-registration";
import type { SignTransaction } from "@stellar/stellar-sdk/contract";

export interface UpdateProfileParams {
  playerId: bigint;
  /**
   * Canonical URI/CID for the player's updated profile metadata blob,
   * already pinned to IPFS by the caller before this is invoked. Stored
   * on-chain as the contract's single-element `ipfs_hashes` list.
   */
  metadataUri: string;
  network: keyof typeof networks;
  rpcUrl: string;
  /** Public key of the player's wallet — must match the registered player. */
  publicKey: string;
  /** Wallet signing callback (e.g. Freighter's `signTransaction`). */
  signTransaction: SignTransaction;
}

export interface UpdateProfileResult {
  transactionId: string;
  metadataUri: string;
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

export async function updateProfile(params: UpdateProfileParams): Promise<UpdateProfileResult> {
  const { playerId, metadataUri, network, rpcUrl, publicKey, signTransaction } = params;

  const client = new RegistrationClient({
    ...networks[network],
    rpcUrl,
    publicKey,
    signTransaction,
  });

  let sent;
  try {
    const assembled = await client.update_profile({
      player_id: playerId,
      ipfs_hashes: [metadataUri],
    });
    sent = await assembled.signAndSend();
  } catch (err) {
    throw new Error(
      `updateProfile: update_profile Soroban transaction failed for player ${playerId}: ` +
        `${err instanceof Error ? err.message : String(err)}`,
      { cause: err instanceof Error ? err : undefined },
    );
  }

  // `update_profile` returns Result<(), ScoutChainError> on the Rust side, so
  // a successfully *submitted* transaction can still carry a contract-level
  // Err (e.g. PlayerNotFound) — check that separately from send/confirm
  // failures above.
  if (isResultLike(sent.result) && sent.result.isErr()) {
    throw new Error(
      `updateProfile: registration contract rejected update_profile for player ` +
        `${playerId}: ${String(sent.result.unwrapErr())}`,
    );
  }

  const transactionId = sent.sendTransactionResponse?.hash;
  if (sent.getTransactionResponse?.status !== "SUCCESS" || !transactionId) {
    throw new Error(
      `updateProfile: transaction for player ${playerId} did not confirm as SUCCESS ` +
        `(status: ${sent.getTransactionResponse?.status ?? "unknown"})`,
    );
  }

  return { transactionId, metadataUri };
}

/**
 * Reference implementation for `approveMilestone()`.
 *
 * The production milestone approval flow consumed by ScoutChain clients lives
 * in the separate `scoutchain-backend` repo (see `../../../ai.md` and
 * `../../../README.md` — "Backend and frontend repos"). This file is the
 * pattern to port into that backend to make milestone approvals submit real
 * on-chain transactions rather than stubs.
 *
 * This file is not imported anywhere in `scout-off-contracts` — it is a
 * reference implementation only. It builds and submits a real
 * `approve_milestone(validator_wallet, player_id, description, evidence_hash)`
 * Soroban transaction via the generated `@scoutchain/bindings-verification`
 * client and returns the on-chain milestone index assigned by the contract
 * (`Result<u32, VerificationError>`) together with the Soroban transaction
 * hash.
 *
 * See `contracts/verification/src/lib.rs::approve_milestone` and
 * `docs/CONTRACT_REFERENCE.md` for the on-chain contract this wraps:
 *
 *   approve_milestone(
 *     validator_wallet: Address,
 *     player_id: u64,
 *     description: String,
 *     evidence_hash: String,
 *   ) -> Result<u32, VerificationError>
 *   Auth: the validator's wallet must sign.
 *   Errors:
 *     ValidatorNotFound (5)      — wallet not in the registry
 *     ValidatorInactive (6)      — validator has been revoked
 *     PlayerNotFound (8)         — invalid player_id
 *     InvalidInput (9)           — evidence_hash not a valid IPFS/Arweave CID,
 *                                  description too long, or other field error
 *     DuplicateEvidence (16)     — evidence_hash already used in a prior approval
 *     ContractPaused (3)         — circuit breaker is active
 *
 * evidence_hash must be a valid IPFS (`Qm…`) or Arweave (`bafy…`) CID of
 * 2–128 bytes. Upload the evidence file to IPFS/Arweave before calling this
 * function, then pass the resulting CID.
 *
 * Note on generated types: the exact shape of `AssembledTransaction<T>.result`
 * for a fallible contract method depends on the pinned `stellar-cli` version
 * used by `scripts/generate-bindings.sh` (currently 25.2.0). This file
 * defensively checks for both shapes — verify against your generated
 * `src/index.ts` and adjust if a newer codegen version changes that convention.
 */
import { Client as VerificationClient, networks } from "@scoutchain/bindings-verification";
import type { SignTransaction } from "@stellar/stellar-sdk/contract";

export interface ApproveMilestoneParams {
  /** Address of the registered, active validator submitting the approval. */
  validatorWallet: string;
  /** Numeric player identifier assigned by the registration contract. */
  playerId: bigint;
  /** Human-readable milestone description (e.g. "Scored 10 goals in U17 season"). */
  description: string;
  /**
   * IPFS (`Qm…`) or Arweave (`bafy…`) CID of the supporting evidence document
   * (video clip, stat sheet, etc.), already uploaded by the caller.
   * Must be 2–128 bytes. The contract rejects duplicate CIDs across all
   * approvals for the same player (`DuplicateEvidence`, code 16).
   */
  evidenceHash: string;
  network: keyof typeof networks;
  rpcUrl: string;
  /** Public key of the validator's wallet — must match a registered, active validator. */
  publicKey: string;
  /** Wallet signing callback (e.g. Freighter's `signTransaction`). */
  signTransaction: SignTransaction;
}

export interface ApproveMilestoneResult {
  /** Soroban transaction hash (64 lowercase hex chars). */
  transactionId: string;
  /** 1-indexed milestone index assigned by the contract. */
  milestoneIndex: number;
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

export async function approveMilestone(
  params: ApproveMilestoneParams,
): Promise<ApproveMilestoneResult> {
  const {
    validatorWallet,
    playerId,
    description,
    evidenceHash,
    network,
    rpcUrl,
    publicKey,
    signTransaction,
  } = params;

  const client = new VerificationClient({
    ...networks[network],
    rpcUrl,
    publicKey,
    signTransaction,
  });

  let sent;
  try {
    const assembled = await client.approve_milestone({
      validator_wallet: validatorWallet,
      player_id: playerId,
      description,
      evidence_hash: evidenceHash,
    });
    sent = await assembled.signAndSend();
  } catch (err) {
    throw new Error(
      `approveMilestone: approve_milestone Soroban transaction failed for player ` +
        `${playerId} (validator: ${validatorWallet}): ` +
        `${err instanceof Error ? err.message : String(err)}`,
      { cause: err instanceof Error ? err : undefined },
    );
  }

  // `approve_milestone` returns Result<u32, VerificationError> on the Rust side,
  // so a successfully *submitted* transaction can still carry a contract-level
  // Err (e.g. ValidatorInactive, DuplicateEvidence) — check that separately
  // from send/confirm failures above.
  if (isResultLike(sent.result) && sent.result.isErr()) {
    throw new Error(
      `approveMilestone: verification contract rejected approve_milestone for player ` +
        `${playerId}: ${String(sent.result.unwrapErr())}`,
    );
  }

  const transactionId = sent.sendTransactionResponse?.hash;
  if (sent.getTransactionResponse?.status !== "SUCCESS" || !transactionId) {
    throw new Error(
      `approveMilestone: transaction for player ${playerId} did not confirm as SUCCESS ` +
        `(status: ${sent.getTransactionResponse?.status ?? "unknown"})`,
    );
  }

  // The contract returns the new milestone index (u32, 1-indexed).
  const rawIndex = isResultLike(sent.result) ? sent.result.unwrap() : sent.result;
  const milestoneIndex = Number(rawIndex);

  return { transactionId, milestoneIndex };
}

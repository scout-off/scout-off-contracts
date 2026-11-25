/**
 * Utilities for normalising AssembledTransaction results from the generated
 * per-contract clients.
 *
 * The Stellar SDK's contract bindings can return either:
 *   - A plain value  (simple return type, no Result wrapper)
 *   - A Result<T,E> with .isOk() / .isErr() / .unwrap() / .unwrapErr()
 *
 * Both shapes can appear in the same codebase; `unwrapResult` handles both.
 */
import {
  ScoutChainContractError,
  ProgressError,
  RegistrationError,
  ScoutAccessError,
  VerificationError,
  ProgressContractError,
  RegistrationContractError,
  ScoutAccessContractError,
  VerificationContractError,
} from "./errors";

type ErrorFactory = (code: number) => ScoutChainContractError;

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

/**
 * Unwrap a contract call result.
 *
 * - If `result` has `.isErr()`, treats it as a Soroban `Result<T,E>`.
 *   - On error, decodes the numeric error code with `errorFactory` and throws
 *     the corresponding typed `ScoutChainContractError` subclass.
 *   - On success, returns the unwrapped value.
 * - If `result` has no `.isErr()`, returns it directly (simple return type).
 */
export function unwrapResult<T>(
  result: unknown,
  errorFactory: ErrorFactory,
  context = "contract call",
): T {
  if (!isResultLike(result)) {
    return result as T;
  }
  if (result.isErr()) {
    const err = result.unwrapErr();
    // The error value from the generated bindings is an object like
    // { message: "Error(Contract, #7)" } or a numeric code wrapped in a
    // scval. Extract the numeric code as best we can.
    const code = extractErrorCode(err);
    if (code !== null) {
      throw errorFactory(code);
    }
    throw new Error(`${context}: contract returned error: ${String(err)}`);
  }
  return result.unwrap() as T;
}

function extractErrorCode(err: unknown): number | null {
  if (typeof err === "number") return err;
  if (typeof err === "bigint") return Number(err);
  // Soroban SDK wraps errors as objects with a message like "Error(Contract, #7)"
  if (typeof err === "object" && err !== null) {
    const msg = String((err as Record<string, unknown>).message ?? "");
    const m = msg.match(/#(\d+)/);
    if (m) return Number(m[1]);
  }
  const str = String(err);
  const m = str.match(/#(\d+)/);
  if (m) return Number(m[1]);
  return null;
}

/** Error factories per contract. */
export const errorFactories = {
  progress: (code: number) => ProgressError.fromCode(code),
  registration: (code: number) => RegistrationError.fromCode(code),
  scoutAccess: (code: number) => ScoutAccessError.fromCode(code),
  verification: (code: number) => VerificationError.fromCode(code),
} satisfies Record<string, ErrorFactory>;

export {
  ScoutChainContractError,
  ProgressContractError,
  RegistrationContractError,
  ScoutAccessContractError,
  VerificationContractError,
};

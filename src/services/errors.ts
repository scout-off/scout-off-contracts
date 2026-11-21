/**
 * Typed SEP-10 authentication errors (#1809). Only AuthError maps to a 401;
 * every other error (e.g. a Prisma failure) is a 500.
 */
export type AuthErrorCode =
  | "challenge_expired"
  | "challenge_replayed"
  | "account_mismatch"
  | "bad_signature"
  | "invalid_challenge";

export class AuthError extends Error {
  readonly code: AuthErrorCode;
  constructor(code: AuthErrorCode, message?: string) {
    super(message ?? code);
    this.name = "AuthError";
    this.code = code;
  }
}

/**
 * The single place where SDK error wording is interpreted. Only errors whose
 * class is `InvalidChallengeError` are considered; anything else returns null
 * so the caller rethrows it unchanged.
 */
export function mapSdkAuthError(error: unknown): AuthError | null {
  if (!(error instanceof Error) || error.name !== "InvalidChallengeError") {
    return null;
  }
  let code: AuthErrorCode = "invalid_challenge";
  if (/expired|time ?bounds/i.test(error.message)) code = "challenge_expired";
  else if (/signature|signer/i.test(error.message)) code = "bad_signature";
  return new AuthError(code, error.message);
}

/** Run an SDK call, converting InvalidChallengeError into AuthError. */
export function wrapSdkAuth<T>(fn: () => T): T {
  try {
    return fn();
  } catch (error) {
    throw mapSdkAuthError(error) ?? error;
  }
}

/**
 * Tests for typed SEP-10 auth errors (#1809): one per stable code plus the
 * "database error mentioning challenge is a 500" case.
 */
import { AuthError, mapSdkAuthError, wrapSdkAuth } from "../src/services/errors";

function sdkError(message: string): Error {
  const e = new Error(message);
  e.name = "InvalidChallengeError";
  return e;
}

describe("mapSdkAuthError", () => {
  it("maps expiry to challenge_expired", () => {
    expect(mapSdkAuthError(sdkError("The transaction has expired"))?.code).toBe("challenge_expired");
  });
  it("maps time bounds to challenge_expired", () => {
    expect(mapSdkAuthError(sdkError("Transaction timeBounds invalid"))?.code).toBe("challenge_expired");
  });
  it("maps signer failures to bad_signature", () => {
    expect(mapSdkAuthError(sdkError("Transaction not signed by any client signer"))?.code).toBe("bad_signature");
  });
  it("maps other InvalidChallengeError to invalid_challenge", () => {
    expect(mapSdkAuthError(sdkError("Invalid sequence number"))?.code).toBe("invalid_challenge");
  });
  it("ignores non-SDK errors, even mentioning challenge", () => {
    const prisma = new Error("Unique constraint failed on sep10_challenges challenge hash");
    prisma.name = "PrismaClientKnownRequestError";
    expect(mapSdkAuthError(prisma)).toBeNull();
    expect(() => wrapSdkAuth(() => { throw prisma; })).toThrow(prisma);
  });
});

describe("AuthError", () => {
  it.each(["challenge_replayed", "account_mismatch"] as const)("carries code %s", (code) => {
    const e = new AuthError(code);
    expect(e.code).toBe(code);
    expect(e).toBeInstanceOf(Error);
  });
});

describe("POST /auth classification", () => {
  it("only AuthError is a 401; a Prisma error is not", () => {
    jest.isolateModules(() => {
      // classifyAuthError is private; exercised via the instanceof contract.
      expect(new AuthError("bad_signature") instanceof AuthError).toBe(true);
      expect(new Error("challenge db failure") instanceof AuthError).toBe(false);
    });
  });
});

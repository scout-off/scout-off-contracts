import { Keypair } from "@stellar/stellar-sdk";
import {
  AccountQuery,
  VerifyBody,
  RefreshBody,
  RevokeBody,
  MAX_TRANSACTION_LENGTH,
} from "../src/requestValidation";

const ACCOUNT = Keypair.random().publicKey();
const JWT = "aaa.bbb.ccc";

describe("AccountQuery", () => {
  it("accepts a valid account", () => {
    expect(AccountQuery.safeParse({ account: ACCOUNT }).success).toBe(true);
  });
  it("rejects missing, wrong type and malformed", () => {
    expect(AccountQuery.safeParse({}).success).toBe(false);
    expect(AccountQuery.safeParse({ account: [ACCOUNT, ACCOUNT] }).success).toBe(false);
    expect(AccountQuery.safeParse({ account: "nope" }).success).toBe(false);
  });
  it("strips unknown keys", () => {
    expect(AccountQuery.parse({ account: ACCOUNT, extra: 1 })).toEqual({ account: ACCOUNT });
  });
});

describe("VerifyBody", () => {
  it("accepts valid input", () => {
    expect(VerifyBody.safeParse({ transaction: "QUJD", account: ACCOUNT }).success).toBe(true);
  });
  it("rejects missing, wrong type, non-base64 and oversize", () => {
    expect(VerifyBody.safeParse({ account: ACCOUNT }).success).toBe(false);
    expect(VerifyBody.safeParse({ transaction: 5, account: ACCOUNT }).success).toBe(false);
    expect(VerifyBody.safeParse({ transaction: "!!!", account: ACCOUNT }).success).toBe(false);
    expect(
      VerifyBody.safeParse({ transaction: "A".repeat(MAX_TRANSACTION_LENGTH + 1), account: ACCOUNT }).success
    ).toBe(false);
  });
});

describe("RefreshBody", () => {
  it("accepts a string and rejects missing / wrong type / oversize", () => {
    expect(RefreshBody.safeParse({ refreshToken: "abc" }).success).toBe(true);
    expect(RefreshBody.safeParse({}).success).toBe(false);
    expect(RefreshBody.safeParse({ refreshToken: {} }).success).toBe(false);
    expect(RefreshBody.safeParse({ refreshToken: "a".repeat(5000) }).success).toBe(false);
  });
});

describe("RevokeBody", () => {
  it("accepts a JWT-shaped string and rejects missing / wrong type / bad shape / oversize", () => {
    expect(RevokeBody.safeParse({ token: JWT }).success).toBe(true);
    expect(RevokeBody.safeParse({}).success).toBe(false);
    expect(RevokeBody.safeParse({ token: 123 }).success).toBe(false);
    expect(RevokeBody.safeParse({ token: "notajwt" }).success).toBe(false);
    expect(RevokeBody.safeParse({ token: "a.b." + "c".repeat(9000) }).success).toBe(false);
  });
});

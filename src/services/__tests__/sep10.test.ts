/**
 * src/services/__tests__/sep10.test.ts
 *
 * Unit tests for the SEP-10 service: JWT issuance claims/expiry and the
 * challenge build/verify round-trip.
 *
 * Run with: npx jest src/services/__tests__/sep10.test.ts
 */

import jwt from "jsonwebtoken";
import { Keypair, Networks, Transaction } from "@stellar/stellar-sdk";

jest.mock("../../db", () => ({
  prisma: {
    revokedToken: { findFirst: jest.fn().mockResolvedValue(null) },
  },
}));

import { issueSep10Token, buildChallengeTx, verifyChallengeTx } from "../sep10";

const SECRET = "test-secret";

beforeAll(() => {
  process.env.SEP10_SIGNING_SECRET = Keypair.random().secret();
  process.env.HOME_DOMAIN = "scoutoff.test";
  process.env.WEB_AUTH_DOMAIN = "auth.scoutoff.test";
  process.env.STELLAR_NETWORK = "testnet";
});

afterEach(() => {
  jest.restoreAllMocks();
});

function signChallenge(challenge: string, kp: Keypair): string {
  const tx = new Transaction(challenge, Networks.TESTNET_PASSPHRASE);
  tx.sign(kp);
  return tx.toXDR();
}

describe("issueSep10Token", () => {
  it("includes jti, sub and role claims", () => {
    const sub = Keypair.random().publicKey();
    const decoded = jwt.verify(
      issueSep10Token({ sub, role: "scout" }, SECRET),
      SECRET
    ) as jwt.JwtPayload;

    expect(decoded.sub).toBe(sub);
    expect(decoded.role).toBe("scout");
    expect(typeof decoded.jti).toBe("string");
    expect(decoded.jti).not.toHaveLength(0);
  });

  it("expires after 7 days", () => {
    const decoded = jwt.verify(
      issueSep10Token({ sub: "G", role: "player" }, SECRET),
      SECRET
    ) as jwt.JwtPayload;

    expect(decoded.exp! - decoded.iat!).toBe(7 * 24 * 60 * 60);
  });

  it("uses a unique jti per token", () => {
    const a = jwt.decode(issueSep10Token({ sub: "G", role: "p" }, SECRET)) as jwt.JwtPayload;
    const b = jwt.decode(issueSep10Token({ sub: "G", role: "p" }, SECRET)) as jwt.JwtPayload;
    expect(a.jti).not.toBe(b.jti);
  });
});

describe("buildChallengeTx / verifyChallengeTx", () => {
  it("verifies a challenge signed by the account it was built for", async () => {
    const clientA = Keypair.random();
    const signed = signChallenge(buildChallengeTx(clientA.publicKey()), clientA);

    await expect(verifyChallengeTx(signed, clientA.publicKey())).resolves.toBe(
      clientA.publicKey()
    );
  });

  it("rejects a challenge built for A when verified for B", async () => {
    const clientA = Keypair.random();
    const clientB = Keypair.random();
    const signed = signChallenge(buildChallengeTx(clientA.publicKey()), clientA);

    await expect(verifyChallengeTx(signed, clientB.publicKey())).rejects.toThrow();
  });

  it("rejects a challenge after it has expired", async () => {
    const clientA = Keypair.random();
    const signed = signChallenge(buildChallengeTx(clientA.publicKey()), clientA);

    const later = Date.now() + 60 * 60 * 1000;
    jest.spyOn(Date, "now").mockReturnValue(later);

    await expect(verifyChallengeTx(signed, clientA.publicKey())).rejects.toThrow();
  });
});

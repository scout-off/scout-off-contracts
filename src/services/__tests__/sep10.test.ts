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

// In-memory stand-in for the single-use challenge store (#1606). updateMany
// applies its `where` and mutation together, like the conditional UPDATE the
// real database performs.
jest.mock("../../db", () => {
  const challenges = new Map<string, any>();
  const matches = (row: any, where: any) =>
    Object.entries(where).every(([k, v]: [string, any]) =>
      v && typeof v === "object" && "gt" in v ? row[k] > v.gt : row[k] === v
    );
  return {
    prisma: {
      revokedToken: { findFirst: jest.fn().mockResolvedValue(null) },
      sep10Challenge: {
        create: async ({ data }: any) => {
          challenges.set(data.hash, { consumedAt: null, ...data });
          return data;
        },
        updateMany: async ({ where, data }: any) => {
          let count = 0;
          for (const row of challenges.values()) {
            if (matches(row, where)) {
              Object.assign(row, data);
              count++;
            }
          }
          return { count };
        },
      },
    },
  };
});

import {
  issueSep10Token,
  buildChallengeTx,
  verifyChallengeTx,
  accessTokenTtlSeconds,
} from "../sep10";

// In test, use HS256 migration mode with a known secret.
const MIGRATION_SECRET =
  "test-jwt-secret-that-is-at-least-32-bytes-long-xxxxxx";
const cfg = {
  signingKeys: null,
  allowHs256Migration: true,
  jwtSecret: MIGRATION_SECRET,
};

beforeAll(() => {
  process.env.SEP10_SIGNING_SECRET = Keypair.random().secret();
  process.env.HOME_DOMAIN = "scoutoff.test";
  process.env.WEB_AUTH_DOMAIN = "auth.scoutoff.test";
  process.env.STELLAR_NETWORK = "testnet";
});

describe("issueSep10Token", () => {
  it("includes jti, sub and role claims", async () => {
    const sub = Keypair.random().publicKey();
    const decoded = jwt.verify(
      await issueSep10Token({ sub, role: "scout" }, cfg),
      MIGRATION_SECRET
    ) as jwt.JwtPayload;

    expect(decoded.sub).toBe(sub);
    expect(decoded.role).toBe("scout");
    expect(typeof decoded.jti).toBe("string");
    // crypto.randomUUID() produces an RFC 4122 v4 UUID.
    expect(decoded.jti).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
    );
  });

  it("expires after the access-token lifetime (capped at 15 minutes)", async () => {
    const decoded = jwt.verify(
      await issueSep10Token({ sub: "G", role: "player" }, cfg),
      MIGRATION_SECRET
    ) as jwt.JwtPayload;

    expect(decoded.exp! - decoded.iat!).toBe(accessTokenTtlSeconds());
    expect(accessTokenTtlSeconds()).toBeLessThanOrEqual(15 * 60);
  });

  it("uses a unique jti per token", async () => {
    const a = jwt.decode(await issueSep10Token({ sub: "G", role: "p" }, cfg)) as jwt.JwtPayload;
    const b = jwt.decode(await issueSep10Token({ sub: "G", role: "p" }, cfg)) as jwt.JwtPayload;
    expect(a.jti).not.toBe(b.jti);
  });
});

afterEach(() => {
  jest.restoreAllMocks();
});

function signChallenge(challenge: string, kp: Keypair): string {
  const tx = new Transaction(challenge, Networks.TESTNET);
  tx.sign(kp);
  return tx.toXDR();
}



describe("buildChallengeTx / verifyChallengeTx", () => {
  it("verifies a challenge signed by the account it was built for", async () => {
    const clientA = Keypair.random();
    const signed = signChallenge(await buildChallengeTx(clientA.publicKey()), clientA);

    await expect(verifyChallengeTx(signed, clientA.publicKey())).resolves.toBe(
      clientA.publicKey()
    );
  });

  it("rejects a challenge built for A when verified for B", async () => {
    const clientA = Keypair.random();
    const clientB = Keypair.random();
    const signed = signChallenge(await buildChallengeTx(clientA.publicKey()), clientA);

    await expect(verifyChallengeTx(signed, clientB.publicKey())).rejects.toThrow();
  });

  it("rejects a challenge after it has expired", async () => {
    const clientA = Keypair.random();
    const signed = signChallenge(await buildChallengeTx(clientA.publicKey()), clientA);

    const later = Date.now() + 60 * 60 * 1000;
    jest.spyOn(Date, "now").mockReturnValue(later);

    await expect(verifyChallengeTx(signed, clientA.publicKey())).rejects.toThrow();
  });
});

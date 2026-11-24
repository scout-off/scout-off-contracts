import request from "supertest";
import jwt from "jsonwebtoken";
import { Keypair, Networks, Transaction, TransactionBuilder } from "@stellar/stellar-sdk";

// In-memory stand-in for the Prisma tables this flow touches. updateMany
// applies its `where` and mutation synchronously, mirroring the atomic
// conditional UPDATE the real database performs.
const db = {
  challenges: new Map<string, any>(),
  refresh: new Map<string, any>(),
  validators: new Map<string, any>(),
};

function matches(row: any, where: any): boolean {
  return Object.entries(where).every(([k, v]: [string, any]) => {
    if (v && typeof v === "object" && "gt" in v) return row[k] > v.gt;
    return row[k] === v;
  });
}

function table(store: Map<string, any>, key: string) {
  return {
    create: async ({ data }: any) => {
      store.set(data[key], { consumedAt: null, revokedAt: null, ...data });
      return data;
    },
    findUnique: async ({ where }: any) => store.get(where[key]) ?? null,
    updateMany: async ({ where, data }: any) => {
      let count = 0;
      for (const row of store.values()) {
        if (matches(row, where)) {
          Object.assign(row, data);
          count++;
        }
      }
      return { count };
    },
  };
}

jest.mock("../src/db", () => ({
  prisma: {
    sep10Challenge: table(db.challenges, "hash"),
    refreshToken: table(db.refresh, "tokenHash"),
    revokedToken: { findFirst: async () => null },
    admin: { findUnique: async () => null },
    validator: {
      findUnique: async ({ where }: any) => db.validators.get(where.wallet) ?? null,
    },
    scout: { findUnique: async () => null },
    player: { findUnique: async () => null },
  },
}));

import { app } from "../src/app";
import { config } from "../src/config";

const server = Keypair.random();
const client = Keypair.random();

beforeAll(() => {
  process.env.SEP10_SIGNING_SECRET = server.secret();
  process.env.HOME_DOMAIN = "scoutchain.com";
  process.env.WEB_AUTH_DOMAIN = "app.scoutchain.com";
  process.env.STELLAR_NETWORK = "testnet";
});

beforeEach(() => {
  db.challenges.clear();
  db.refresh.clear();
  db.validators.clear();
  delete process.env.ACCESS_TOKEN_TTL_SECONDS;
});

async function getChallenge(): Promise<string> {
  const res = await request(app)
    .get("/auth")
    .query({ account: client.publicKey() })
    .expect(200);
  return res.body.challenge;
}

function sign(challenge: string): string {
  const tx = TransactionBuilder.fromXDR(challenge, Networks.TESTNET);
  tx.sign(client);
  return tx.toXDR();
}

const login = (transaction: string) =>
  request(app).post("/auth").send({ transaction, account: client.publicKey() });

describe("SEP-10 challenge", () => {
  it("starts at the server's current time, not epoch 0", async () => {
    const before = Math.floor(Date.now() / 1000);
    const challenge = await getChallenge();
    const tx = TransactionBuilder.fromXDR(challenge, Networks.TESTNET) as Transaction;
    const { minTime, maxTime } = tx.timeBounds!;

    expect(Number(minTime)).toBeGreaterThanOrEqual(before - 1);
    expect(Number(minTime)).toBeLessThanOrEqual(before + 5);
    expect(Number(maxTime) - Number(minTime)).toBe(300);
  });

  it("rejects a second submission of the same signed challenge with 401", async () => {
    const signed = sign(await getChallenge());

    await login(signed).expect(200);
    await login(signed).expect(401);
  });

  it("rejects a challenge the server never issued", async () => {
    const signed = sign(await getChallenge());
    db.challenges.clear();

    await login(signed).expect(401);
  });

  it("does not consume the challenge when the client signature is missing", async () => {
    const challenge = await getChallenge();

    await login(challenge).expect(401);
    await login(sign(challenge)).expect(200);
  });
});

describe("session tokens", () => {
  it("issues access tokens that live at most 15 minutes", async () => {
    process.env.ACCESS_TOKEN_TTL_SECONDS = "86400";
    const res = await login(sign(await getChallenge())).expect(200);

    const decoded: any = jwt.verify(res.body.token, config.jwtSecret);
    expect(decoded.exp - decoded.iat).toBeLessThanOrEqual(900);
    expect(res.body.refreshToken).toBeDefined();
  });

  it("honours a shorter configured access-token lifetime", async () => {
    process.env.ACCESS_TOKEN_TTL_SECONDS = "60";
    const res = await login(sign(await getChallenge())).expect(200);

    const decoded: any = jwt.verify(res.body.token, config.jwtSecret);
    expect(decoded.exp - decoded.iat).toBe(60);
  });

  it("re-derives the role on refresh and rotates the refresh token", async () => {
    db.validators.set(client.publicKey(), { wallet: client.publicKey(), active: true });
    const first = await login(sign(await getChallenge())).expect(200);
    expect((jwt.decode(first.body.token) as any).role).toBe("validator");

    // The validator is revoked after login.
    db.validators.set(client.publicKey(), { wallet: client.publicKey(), active: false });

    const refreshed = await request(app)
      .post("/auth/refresh")
      .send({ refreshToken: first.body.refreshToken })
      .expect(200);
    expect((jwt.decode(refreshed.body.token) as any).role).toBe("unknown");
    expect(refreshed.body.refreshToken).not.toBe(first.body.refreshToken);

    // The rotated-out token cannot be used again.
    await request(app)
      .post("/auth/refresh")
      .send({ refreshToken: first.body.refreshToken })
      .expect(401);
  });

  it("rejects an unknown refresh token", async () => {
    await request(app).post("/auth/refresh").send({ refreshToken: "nope" }).expect(401);
    await request(app).post("/auth/refresh").send({}).expect(400);
  });
});

// Integration test: full SEP-10 flow against a real Postgres database.
// Runs when DATABASE_URL is set (the CI backend job provides a Postgres
// service with the schema applied); skipped otherwise. A read-then-write
// regression in challenge/refresh consumption makes the concurrency tests fail.
import request from "supertest";
import { Keypair, Networks, TransactionBuilder } from "@stellar/stellar-sdk";
import { PrismaClient } from "@prisma/client";

const enabled = !!process.env.DATABASE_URL;
const describeDb = enabled ? describe : describe.skip;

const server = Keypair.random();
const client = Keypair.random();
const CONCURRENCY = 10;

describeDb("SEP-10 flow against Postgres", () => {
  let app: any;
  let prisma: PrismaClient;

  beforeAll(async () => {
    process.env.SEP10_SIGNING_SECRET = server.secret();
    process.env.HOME_DOMAIN = "scoutchain.com";
    process.env.WEB_AUTH_DOMAIN = "app.scoutchain.com";
    process.env.STELLAR_NETWORK = "testnet";
    ({ app } = await import("../src/app"));
    ({ prisma } = await import("../src/db"));
  });

  beforeEach(async () => {
    await prisma.refreshToken.deleteMany();
    await prisma.sep10Challenge.deleteMany();
  });

  afterAll(async () => {
    await prisma.$disconnect();
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

  const refresh = (refreshToken: string) =>
    request(app).post("/auth/refresh").send({ refreshToken });

  it("happy path persists a challenge and a refresh token", async () => {
    const challenge = await getChallenge();
    const rows = await prisma.sep10Challenge.findMany();
    expect(rows).toHaveLength(1);
    expect(rows[0].account).toBe(client.publicKey());
    expect(rows[0].consumedAt).toBeNull();

    const res = await login(sign(challenge)).expect(200);
    expect(res.body.token).toBeDefined();
    expect(res.body.refreshToken).toBeDefined();

    const consumed = await prisma.sep10Challenge.findMany();
    expect(consumed[0].consumedAt).not.toBeNull();
    const tokens = await prisma.refreshToken.findMany();
    expect(tokens).toHaveLength(1);
    expect(tokens[0].account).toBe(client.publicKey());
    expect(tokens[0].revokedAt).toBeNull();
  });

  it("rejects a sequential replay with challenge_replayed", async () => {
    const signed = sign(await getChallenge());
    await login(signed).expect(200);
    const res = await login(signed).expect(401);
    expect(res.body).toEqual({ error: "challenge_replayed" });
  });

  it("lets exactly one of concurrent identical submissions succeed", async () => {
    const signed = sign(await getChallenge());
    const results = await Promise.all(
      Array.from({ length: CONCURRENCY }, () => login(signed))
    );
    expect(results.filter((r) => r.status === 200)).toHaveLength(1);
    expect(results.filter((r) => r.status === 401)).toHaveLength(CONCURRENCY - 1);
    expect(await prisma.refreshToken.count()).toBe(1);
  });

  it("lets exactly one of concurrent refreshes of the same token succeed", async () => {
    const first = await login(sign(await getChallenge())).expect(200);
    const results = await Promise.all(
      Array.from({ length: CONCURRENCY }, () => refresh(first.body.refreshToken))
    );
    expect(results.filter((r) => r.status === 200)).toHaveLength(1);
    expect(results.filter((r) => r.status === 401)).toHaveLength(CONCURRENCY - 1);
  });

  it("rejects an expired challenge", async () => {
    const signed = sign(await getChallenge());
    await prisma.sep10Challenge.updateMany({
      data: { expiresAt: new Date(Date.now() - 60_000) },
    });
    await login(signed).expect(401);
    expect(await prisma.refreshToken.count()).toBe(0);
  });
});

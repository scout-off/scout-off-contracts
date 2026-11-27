import { randomBytes } from "node:crypto";
import jwt from "jsonwebtoken";
import request from "supertest";
import { Keypair } from "@stellar/stellar-base";
import sodium from "libsodium-wrappers";
import { createApp } from "../src/app";
import type { EvidenceDatabase } from "../src/routes/evidence";
import {
  decryptEvidenceDataKey,
  encryptEvidenceDataKey,
} from "../src/services/evidenceKeyCrypto";

const jwtSecret = "evidence-test-secret";
const masterKey = randomBytes(32);
const playerId = "42";
const milestoneIndex = "3";

function createHarness(options?: {
  grant?: boolean;
  playerWallet?: string;
  auditError?: boolean;
}) {
  const scout = Keypair.random();
  const player = Keypair.random();
  const dataKey = randomBytes(32);
  const encrypted = encryptEvidenceDataKey(
    dataKey,
    masterKey,
    playerId,
    milestoneIndex
  );
  const released: Array<{ wallet: string; role: string }> = [];
  const queries: string[] = [];
  let grant = options?.grant ?? true;

  const database: EvidenceDatabase = {
    async query<Row>(text: string, values?: unknown[]) {
      queries.push(text);
      if (text.includes("SELECT wallet FROM players")) {
        return {
          rows: [{ wallet: options?.playerWallet ?? player.publicKey() } as Row],
          rowCount: 1,
        };
      }
      if (text.includes("FROM evidence_data_keys")) {
        return {
          rows: [
            {
              encrypted_data_key: encrypted.ciphertext,
              nonce: encrypted.nonce,
              auth_tag: encrypted.authTag,
            } as Row,
          ],
          rowCount: 1,
        };
      }
      if (text.includes("INSERT INTO evidence_key_releases")) {
        if (options?.auditError) {
          throw new Error("audit database unavailable");
        }
        released.push({
          wallet: String(values?.[2]),
          role: String(values?.[3]),
        });
        return { rows: [], rowCount: 1 };
      }
      throw new Error("Unexpected database query");
    },
  };

  const app = createApp({
    database,
    grantChecker: {
      hasEvidenceAccess: async () => grant,
    },
    masterKey,
    jwtSecret,
    isTokenRevoked: async () => false,
  });

  function token(sub: string, role: string, expiresIn = 3600) {
    return jwt.sign({ sub, role }, jwtSecret, {
      algorithm: "HS256",
      jwtid: randomBytes(16).toString("hex"),
      expiresIn,
    });
  }

  return {
    app,
    dataKey,
    scout,
    player,
    released,
    queries,
    token,
    setGrant: (value: boolean) => {
      grant = value;
    },
  };
}

describe("evidence key wrapping", () => {
  it("wraps a key to a Stellar wallet that can unwrap it", async () => {
    await sodium.ready;
    const h = createHarness();
    const response = await request(h.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set("Authorization", `Bearer ${h.token(h.scout.publicKey(), "scout")}`)
      .expect(200);

    expect(response.body.recipient).toBe(h.scout.publicKey());
    expect(response.body.wrappedKey).not.toBe(h.dataKey.toString("base64"));
    expect(h.released).toEqual([
      { wallet: h.scout.publicKey(), role: "scout" },
    ]);

    const ed25519Keys = sodium.crypto_sign_seed_keypair(h.scout.rawSecretKey());
    const curvePublicKey = sodium.crypto_sign_ed25519_pk_to_curve25519(
      ed25519Keys.publicKey
    );
    const curveSecretKey = sodium.crypto_sign_ed25519_sk_to_curve25519(
      ed25519Keys.privateKey
    );
    const unwrapped = sodium.crypto_box_seal_open(
      Buffer.from(response.body.wrappedKey, "base64"),
      curvePublicKey,
      curveSecretKey
    );
    expect(Buffer.from(unwrapped)).toEqual(h.dataKey);
  });

  it("denies a revoked or otherwise absent on-chain grant without auditing a release", async () => {
    const h = createHarness({ grant: false });
    await request(h.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set("Authorization", `Bearer ${h.token(h.scout.publicKey(), "scout")}`)
      .expect(403);

    expect(h.released).toHaveLength(0);
    expect(h.queries.some((query) => query.includes("evidence_data_keys"))).toBe(
      false
    );
  });

  it("rechecks the on-chain grant on every request", async () => {
    const h = createHarness();
    const authorization = `Bearer ${h.token(h.scout.publicKey(), "scout")}`;

    await request(h.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set("Authorization", authorization)
      .expect(200);
    h.setGrant(false);
    await request(h.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set("Authorization", authorization)
      .expect(403);

    expect(h.released).toHaveLength(1);
  });

  it("does not return a key if the release audit cannot be persisted", async () => {
    const h = createHarness({ auditError: true });
    const response = await request(h.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set("Authorization", `Bearer ${h.token(h.scout.publicKey(), "scout")}`)
      .expect(500);

    expect(response.body).toEqual({ error: "Internal server error" });
    expect(JSON.stringify(response.body)).not.toContain(
      h.dataKey.toString("base64")
    );
  });

  it("rejects an expired SEP-10 JWT", async () => {
    const h = createHarness();
    await request(h.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set(
        "Authorization",
        `Bearer ${h.token(h.scout.publicKey(), "scout", -1)}`
      )
      .expect(401);

    expect(h.released).toHaveLength(0);
  });

  it("denies a scout without access to the requested player", async () => {
    const h = createHarness({ grant: false });
    await request(h.app)
      .post("/api/evidence/43/3/key")
      .set("Authorization", `Bearer ${h.token(h.scout.publicKey(), "scout")}`)
      .expect(403);
  });

  it("rejects authenticated non-scouts", async () => {
    const h = createHarness();
    await request(h.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set(
        "Authorization",
        `Bearer ${h.token(h.scout.publicKey(), "validator")}`
      )
      .expect(403);

    expect(h.released).toHaveLength(0);
  });

  it("allows a player to fetch their own key and rejects another player's wallet", async () => {
    const h = createHarness();
    await request(h.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set("Authorization", `Bearer ${h.token(h.player.publicKey(), "player")}`)
      .expect(200);

    expect(h.released).toEqual([
      { wallet: h.player.publicKey(), role: "player" },
    ]);

    const otherOwner = Keypair.random().publicKey();
    const wrongOwner = createHarness({ playerWallet: otherOwner });
    await request(wrongOwner.app)
      .post(`/api/evidence/${playerId}/${milestoneIndex}/key`)
      .set(
        "Authorization",
        `Bearer ${wrongOwner.token(wrongOwner.player.publicKey(), "player")}`
      )
      .expect(403);
    expect(wrongOwner.released).toHaveLength(0);
  });

  it("binds encrypted keys to their player and milestone", () => {
    const key = randomBytes(32);
    const encrypted = encryptEvidenceDataKey(key, masterKey, playerId, milestoneIndex);
    expect(
      decryptEvidenceDataKey(encrypted, masterKey, playerId, milestoneIndex)
    ).toEqual(key);
    expect(() =>
      decryptEvidenceDataKey(encrypted, masterKey, "43", milestoneIndex)
    ).toThrow();
  });
});

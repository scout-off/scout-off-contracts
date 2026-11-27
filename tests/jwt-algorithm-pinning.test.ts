/**
 * #1705 / #1779 JWT algorithm pinning – asymmetric (EdDSA) signing.
 *
 * Tests:
 *  - Tokens are issued with EdDSA and carry a kid header.
 *  - Tokens signed by a previous (rotated-out but still-known) key verify OK.
 *  - Tokens with an unknown kid are rejected.
 *  - alg:none, HS256 confusion, and HS384/HS512 are all rejected.
 *  - A token with no kid header is rejected.
 */
import jwt from "jsonwebtoken";
import { generateKeyPairSync } from "crypto";
import { SignJWT as JoseSignJWT } from "jose";

// ---------------------------------------------------------------------------
// Key material – generated once for the whole suite.
// ---------------------------------------------------------------------------
const { privateKey: activePriv, publicKey: activePub } = generateKeyPairSync("ed25519");
const { privateKey: oldPriv, publicKey: oldPub } = generateKeyPairSync("ed25519");
const { privateKey: unknownPriv } = generateKeyPairSync("ed25519");

// ---------------------------------------------------------------------------
// Mocks (must be declared before importing the modules under test)
// ---------------------------------------------------------------------------
jest.mock("../src/db", () => ({
  prisma: { revokedToken: { findFirst: jest.fn().mockResolvedValue(null) } },
}));

// Mock config with the same key pairs used for signing in the tests.
// The factory re-creates the same key material via require('crypto'), but
// that would produce different keys — instead we pass them through a module
// variable set before jest.mock runs. Simplest approach: build the mock
// config object in the factory using keys generated there, and expose them.
jest.mock("../src/config", () => {
  const { generateKeyPairSync: gkp } = require("crypto");
  const kp1 = gkp("ed25519");
  const kp2 = gkp("ed25519");
  const fakeConfig = {
    signingKeys: {
      activeKid: "key-1",
      activePrivateKey: kp1.privateKey,
      publicKeys: new Map([
        ["key-1", kp1.publicKey],
        ["key-old", kp2.publicKey],
      ]),
      algorithm: "EdDSA",
    },
    allowHs256Migration: false,
    jwtSecret: "",
  };
  return { config: fakeConfig, __kp1: kp1, __kp2: kp2 };
});

// Import AFTER mocks are set up.
import { issueSep10Token } from "../src/services/sep10";
import { requireAuth } from "../src/middleware/auth";

// eslint-disable-next-line @typescript-eslint/no-require-imports
const configMod = require("../src/config") as {
  config: {
    signingKeys: any;
    allowHs256Migration: boolean;
    jwtSecret: string;
  };
  __kp1: { privateKey: any; publicKey: any };
  __kp2: { privateKey: any; publicKey: any };
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function requestFor(token: string): any {
  return { headers: { authorization: `Bearer ${token}` } };
}
function responseMock(): any {
  return { status: jest.fn().mockReturnThis(), json: jest.fn().mockReturnThis() };
}

const claims = { sub: "GTESTACCOUNT", role: "player", jti: "unit-test-jti" };

/** Sign an EdDSA token with jose using the given key and kid. */
async function eddsaToken(
  priv: any,
  kid?: string
): Promise<string> {
  let builder = new JoseSignJWT(claims).setProtectedHeader({ alg: "EdDSA" });
  if (kid) {
    builder = new JoseSignJWT(claims).setProtectedHeader({ alg: "EdDSA", kid });
  }
  return builder
    .setIssuedAt()
    .setExpirationTime("15m")
    .setJti("unit-test-jti")
    .sign(priv);
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------
describe("#1779 JWT asymmetric signing & algorithm pinning", () => {
  it("issueSep10Token signs with EdDSA and includes kid in the header", async () => {
    const token = await issueSep10Token(
      { sub: claims.sub, role: claims.role },
      configMod.config
    );
    const parts = token.split(".");
    const header = JSON.parse(Buffer.from(parts[0], "base64url").toString());
    expect(header.alg).toBe("EdDSA");
    expect(header.kid).toBe("key-1");
  });

  it("accepts a valid EdDSA JWT signed by the active key", async () => {
    const token = await eddsaToken(configMod.__kp1.privateKey, "key-1");
    const response = responseMock();
    const next = jest.fn();
    await requireAuth(requestFor(token), response, next);
    expect(next).toHaveBeenCalledTimes(1);
    expect(response.status).not.toHaveBeenCalled();
  });

  it("accepts an EdDSA JWT signed by a previous (rotated) key still in the key set (overlap)", async () => {
    const token = await eddsaToken(configMod.__kp2.privateKey, "key-old");
    const response = responseMock();
    const next = jest.fn();
    await requireAuth(requestFor(token), response, next);
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("rejects an EdDSA JWT with an unknown kid", async () => {
    const token = await eddsaToken(unknownPriv, "key-unknown");
    const response = responseMock();
    const next = jest.fn();
    await requireAuth(requestFor(token), response, next);
    expect(next).not.toHaveBeenCalled();
    expect(response.status).toHaveBeenCalledWith(401);
    expect(response.json).toHaveBeenCalledWith({
      error: "Unauthorized - invalid token",
    });
  });

  it("rejects a JWT with alg:none", async () => {
    // jwt.sign does not support alg:none; craft the token manually.
    const header = Buffer.from(JSON.stringify({ alg: "none", typ: "JWT" }))
      .toString("base64url");
    const payload = Buffer.from(
      JSON.stringify({ ...claims, iat: Math.floor(Date.now() / 1000) })
    ).toString("base64url");
    const token = `${header}.${payload}.`;
    const response = responseMock();
    const next = jest.fn();
    await requireAuth(requestFor(token), response, next);
    expect(next).not.toHaveBeenCalled();
    expect(response.status).toHaveBeenCalledWith(401);
  });

  it("rejects a JWT signed with HS256 when asymmetric mode is active", async () => {
    const secret = "supersecret-hs256-key-that-is-long-enough-for-hs256";
    const token = jwt.sign(claims, secret, { algorithm: "HS256" });
    const response = responseMock();
    const next = jest.fn();
    await requireAuth(requestFor(token), response, next);
    expect(next).not.toHaveBeenCalled();
    expect(response.status).toHaveBeenCalledWith(401);
  });

  it.each(["HS384", "HS512"] as const)(
    "rejects a JWT signed with %s",
    async (algorithm) => {
      const secret = "supersecret-hs256-key-that-is-long-enough-for-hs256";
      const token = jwt.sign(claims, secret, { algorithm });
      const response = responseMock();
      const next = jest.fn();
      await requireAuth(requestFor(token), response, next);
      expect(next).not.toHaveBeenCalled();
      expect(response.status).toHaveBeenCalledWith(401);
      expect(response.json).toHaveBeenCalledWith({
        error: "Unauthorized - invalid token",
      });
    }
  );

  it("rejects a token with no kid header", async () => {
    // Sign without a kid so the header has none.
    const token = await eddsaToken(configMod.__kp1.privateKey);
    const response = responseMock();
    const next = jest.fn();
    await requireAuth(requestFor(token), response, next);
    expect(next).not.toHaveBeenCalled();
    expect(response.status).toHaveBeenCalledWith(401);
  });
});

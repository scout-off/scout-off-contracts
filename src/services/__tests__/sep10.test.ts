/**
 * src/services/__tests__/sep10.test.ts
 *
 * Unit tests for issueSep10Token / verifySep10Token.
 * Covers: expired token, wrong issuer, wrong audience, and happy path.
 *
 * Run with: npx jest src/services/__tests__/sep10.test.ts
 */

import jwt from "jsonwebtoken";
import {
  issueSep10Token,
  verifySep10Token,
  getJwtConfig,
} from "../sep10.js";

const VALID_ADDRESS = "GABC1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ123456";

// ---------------------------------------------------------------------------
// Helper — sign a token that fails in specific ways
// ---------------------------------------------------------------------------

function signCustom(
  overrides: {
    secret?: string;
    iss?: string;
    aud?: string;
    expiresIn?: number;
    sub?: string;
  } = {}
): string {
  const config = getJwtConfig();
  return jwt.sign(
    { sub: overrides.sub ?? VALID_ADDRESS },
    overrides.secret ?? config.secret,
    {
      expiresIn: overrides.expiresIn ?? config.ttlSeconds,
      issuer: overrides.iss ?? config.issuer,
      audience: overrides.aud ?? config.audience,
    }
  );
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

describe("issueSep10Token / verifySep10Token — happy path", () => {
  it("issues and verifies a valid token", () => {
    const token = issueSep10Token(VALID_ADDRESS);
    const payload = verifySep10Token(token);

    expect(payload.sub).toBe(VALID_ADDRESS);
    expect(payload.iss).toBe(getJwtConfig().issuer);
    expect(payload.aud).toBe(getJwtConfig().audience);
    expect(payload.exp).toBeGreaterThan(Date.now() / 1000);
  });

  it("encodes the stellar address as sub", () => {
    const address = "GBXYZ9999999999999999999999999999999999999999999999999";
    const token = issueSep10Token(address);
    const payload = verifySep10Token(token);
    expect(payload.sub).toBe(address);
  });
});

// ---------------------------------------------------------------------------
// Rejection: expired token
// ---------------------------------------------------------------------------

describe("verifySep10Token — expired token", () => {
  it("throws TokenExpiredError for a token with expiresIn: 0", () => {
    // Set exp to 1 second in the past by setting expiresIn to -1
    const expiredToken = jwt.sign(
      { sub: VALID_ADDRESS },
      getJwtConfig().secret,
      {
        expiresIn: -1, // already expired
        issuer: getJwtConfig().issuer,
        audience: getJwtConfig().audience,
      }
    );

    expect(() => verifySep10Token(expiredToken)).toThrow(
      jwt.TokenExpiredError
    );
  });
});

// ---------------------------------------------------------------------------
// Rejection: wrong issuer
// ---------------------------------------------------------------------------

describe("verifySep10Token — wrong issuer", () => {
  it("throws JsonWebTokenError when iss does not match", () => {
    const token = signCustom({ iss: "some-other-service" });

    expect(() => verifySep10Token(token)).toThrow(jwt.JsonWebTokenError);
  });
});

// ---------------------------------------------------------------------------
// Rejection: wrong audience
// ---------------------------------------------------------------------------

describe("verifySep10Token — wrong audience", () => {
  it("throws JsonWebTokenError when aud does not match", () => {
    const token = signCustom({ aud: "wrong-audience" });

    expect(() => verifySep10Token(token)).toThrow(jwt.JsonWebTokenError);
  });
});

// ---------------------------------------------------------------------------
// Rejection: bad signature
// ---------------------------------------------------------------------------

describe("verifySep10Token — bad signature", () => {
  it("throws JsonWebTokenError when signed with a different secret", () => {
    const token = signCustom({ secret: "totally-different-secret" });

    expect(() => verifySep10Token(token)).toThrow(jwt.JsonWebTokenError);
  });
});

// ---------------------------------------------------------------------------
// issueSep10Token — input validation
// ---------------------------------------------------------------------------

describe("issueSep10Token — input validation", () => {
  it("throws if stellarAddress does not start with G", () => {
    expect(() => issueSep10Token("notastellaraddress")).toThrow(
      /valid Stellar G-address/
    );
  });

  it("throws if stellarAddress is empty string", () => {
    expect(() => issueSep10Token("")).toThrow(/valid Stellar G-address/);
  });
});

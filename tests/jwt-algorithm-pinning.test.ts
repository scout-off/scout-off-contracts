import jwt from "jsonwebtoken";
import { issueSep10Token } from "../src/services/sep10";
import { requireAuth } from "../src/middleware/auth";
import { config } from "../src/config";

jest.mock("../src/db", () => ({
  prisma: { revokedToken: { findFirst: jest.fn().mockResolvedValue(null) } },
}));

// The middleware verifies with the secret loaded by src/config at import time.
const secret = config.jwtSecret;
const claims = { sub: "GTESTACCOUNT", role: "player", jti: "unit-test-jti" };

function requestFor(token: string): any {
  return { headers: { authorization: `Bearer ${token}` } };
}
function responseMock(): any {
  return { status: jest.fn().mockReturnThis(), json: jest.fn().mockReturnThis() };
}

describe("#1705 JWT algorithm pinning", () => {
  it("issues SEP-10 JWTs with an explicit HS256 header", () => {
    const token = issueSep10Token({ sub: claims.sub, role: claims.role }, secret);
    const decoded = jwt.decode(token, { complete: true });
    expect(decoded && typeof decoded !== "string" ? decoded.header.alg : null).toBe("HS256");
  });

  it("accepts a valid HS256 JWT", async () => {
    const token = jwt.sign(claims, secret, { algorithm: "HS256" });
    const response = responseMock();
    const next = jest.fn();
    await requireAuth(requestFor(token), response, next);
    expect(next).toHaveBeenCalledTimes(1);
    expect(response.status).not.toHaveBeenCalled();
  });

  it.each(["HS384", "HS512"] as const)("rejects a JWT signed with %s", async (algorithm) => {
    const token = jwt.sign(claims, secret, { algorithm });
    const response = responseMock();
    const next = jest.fn();
    await requireAuth(requestFor(token), response, next);
    expect(next).not.toHaveBeenCalled();
    expect(response.status).toHaveBeenCalledWith(401);
    expect(response.json).toHaveBeenCalledWith({ error: "Unauthorized - invalid token" });
  });
});

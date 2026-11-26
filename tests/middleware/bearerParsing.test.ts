import jwt from "jsonwebtoken";
import { requireAuth } from "../../src/middleware/auth";
import { config } from "../../src/config";

jest.mock("../../src/db", () => ({
  prisma: { revokedToken: { findFirst: jest.fn().mockResolvedValue(null) } },
}));

const token = jwt.sign(
  { sub: "GTESTACCOUNT", role: "player", jti: "bearer-parse-jti" },
  config.jwtSecret,
  { algorithm: "HS256" }
);

async function run(header?: string) {
  const res: any = {
    set: jest.fn().mockReturnThis(),
    status: jest.fn().mockReturnThis(),
    json: jest.fn().mockReturnThis(),
  };
  const next = jest.fn();
  await requireAuth({ headers: header === undefined ? {} : { authorization: header } } as any, res, next);
  return { res, next };
}

describe("#1867 requireAuth Bearer parsing", () => {
  it.each([
    ["canonical", `Bearer ${token}`],
    ["lowercase scheme", `bearer ${token}`],
    ["uppercase scheme", `BEARER ${token}`],
    ["double space", `Bearer  ${token}`],
    ["surrounding whitespace", `  Bearer ${token}  `],
  ])("accepts %s", async (_n, header) => {
    const { res, next } = await run(header);
    expect(next).toHaveBeenCalledTimes(1);
    expect(res.status).not.toHaveBeenCalled();
  });

  it.each([
    ["extra segments", `Bearer ${token} extra`],
    ["non-JWT token", "Bearer not-a-jwt"],
    ["empty token", "Bearer "],
    ["scheme only", "Bearer"],
  ])("rejects %s as malformed", async (_n, header) => {
    const { res, next } = await run(header);
    expect(next).not.toHaveBeenCalled();
    expect(res.status).toHaveBeenCalledWith(401);
    if (header.trim() !== "Bearer") {
      expect(res.set).toHaveBeenCalledWith("WWW-Authenticate", 'Bearer error="invalid_request"');
    }
  });

  it("rejects a missing header", async () => {
    const { res, next } = await run();
    expect(next).not.toHaveBeenCalled();
    expect(res.status).toHaveBeenCalledWith(401);
  });
});

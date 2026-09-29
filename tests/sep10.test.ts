import request from "supertest";
import jwt from "jsonwebtoken";
import { prisma } from "../../src/db";
import { deriveRole, issueSep10Token } from "../../src/services/sep10";
import app from "../../src/app";

// Mock the Stellar SDK WebAuth methods used by app.ts routes.
jest.mock("@stellar/stellar-sdk", () => ({
  ...jest.requireActual("@stellar/stellar-sdk"),
  WebAuth: {
    buildChallengeTx: jest.fn().mockReturnValue("CHALLENGE_XDR"),
    readChallengeTx: jest.fn().mockReturnValue("GABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF12"),
    verifyChallengeTxSigners: jest.fn().mockReturnValue([]),
  },
}));

// Mock the Prisma client methods used for role derivation.
jest.mock("../../src/db", () => ({
  prisma: {
    revokedToken: {
      findFirst: jest.fn(),
      create: jest.fn(),
      deleteMany: jest.fn(),
    },
    player: { findUnique: jest.fn() },
    scout: { findUnique: jest.fn() },
    validator: { findUnique: jest.fn() },
    admin: { findUnique: jest.fn() },
  },
}));

const mockedPrisma = prisma as unknown as Record<string, Record<string, jest.Mock>>;

beforeAll(() => {
  process.env.SEP10_SIGNING_SECRET =
    "SABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz1234567890AB";
  process.env.HOME_DOMAIN = "scoutchain.com";
  process.env.WEB_AUTH_DOMAIN = "app.scoutchain.com";
  process.env.JWT_SECRET = "test-jwt-secret";
  process.env.STELLAR_NETWORK = "testnet";
});

beforeEach(async () => {
  jest.clearAllMocks();
  await mockedPrisma.revokedToken.deleteMany();
});

afterAll(async () => {
  await mockedPrisma.$disconnect?.();
});

describe("GET /auth", () => {
  it("returns a challenge transaction for a valid account", async () => {
    const res = await request(app)
      .get("/auth")
      .query({ account: "GABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF12" })
      .expect(200);

    expect(res.body.challenge).toBe("CHALLENGE_XDR");
  });

  it("returns 400 when account query parameter is missing", async () => {
    await request(app)
      .get("/auth")
      .expect(400)
      .expect((res: any) => {
        expect(res.body.error).toContain("account");
      });
  });
});

describe("POST /auth", () => {
  it("issues a JWT whose sub is the verified account", async () => {
    const account = "GABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF12";

    const res = await request(app)
      .post("/auth")
      .send({ transaction: "CHALLENGE_XDR", account })
      .expect(200);

    const decoded: any = jwt.verify(res.body.token, process.env.JWT_SECRET!);
    expect(decoded.sub).toBe(account);
  });

  it("issues a JWT whose role is derived from on-chain state (player)", async () => {
    mockedPrisma.player.findUnique.mockResolvedValue({ wallet: "GPLAYER123" });

    const account = "GPLAYER123";
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "CHALLENGE_XDR", account })
      .expect(200);

    const decoded: any = jwt.verify(res.body.token, process.env.JWT_SECRET!);
    expect(decoded.role).toBe("player");
  });

  it("issues a JWT whose role is derived from on-chain state (scout)", async () => {
    mockedPrisma.scout.findUnique.mockResolvedValue({ wallet: "GSCOUT123" });

    const account = "GSCOUT123";
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "CHALLENGE_XDR", account })
      .expect(200);

    const decoded: any = jwt.verify(res.body.token, process.env.JWT_SECRET!);
    expect(decoded.role).toBe("scout");
  });

  it("issues a JWT whose role is derived from on-chain state (validator)", async () => {
    mockedPrisma.validator.findUnique.mockResolvedValue({
      wallet: "GVALID123",
      active: true,
    });

    const account = "GVALID123";
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "CHALLENGE_XDR", account })
      .expect(200);

    const decoded: any = jwt.verify(res.body.token, process.env.JWT_SECRET!);
    expect(decoded.role).toBe("validator");
  });

  it("issues a JWT whose role is derived from on-chain state (admin)", async () => {
    mockedPrisma.admin.findUnique.mockResolvedValue({ address: "GADMIN123" });

    const account = "GADMIN123";
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "CHALLENGE_XDR", account })
      .expect(200);

    const decoded: any = jwt.verify(res.body.token, process.env.JWT_SECRET!);
    expect(decoded.role).toBe("admin");
  });

  it("issues a JWT whose role is unknown for an unrecognised account", async () => {
    mockedPrisma.admin.findUnique.mockResolvedValue(null);
    mockedPrisma.validator.findUnique.mockResolvedValue(null);
    mockedPrisma.scout.findUnique.mockResolvedValue(null);
    mockedPrisma.player.findUnique.mockResolvedValue(null);

    const account = "GUNKNOWN123";
    const res = await request(app)
      .post("/auth")
      .send({ transaction: "CHALLENGE_XDR", account })
      .expect(200);

    const decoded: any = jwt.verify(res.body.token, process.env.JWT_SECRET!);
    expect(decoded.role).toBe("unknown");
  });

  it("returns 400 when transaction or account is missing", async () => {
    await request(app)
      .post("/auth")
      .send({ transaction: "CHALLENGE_XDR" })
      .expect(400)
      .expect((res: any) => {
        expect(res.body.error).toContain("required");
      });

    await request(app)
      .post("/auth")
      .send({ account: "GABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF12" })
      .expect(400)
      .expect((res: any) => {
        expect(res.body.error).toContain("required");
      });
  });

  it("returns 401 for an invalid signature", async () => {
    const { WebAuth } = jest.requireMock("@stellar/stellar-sdk");
    WebAuth.readChallengeTx.mockImplementation(() => {
      throw new Error("Invalid signature");
    });

    await request(app)
      .post("/auth")
      .send({
        transaction: "CHALLENGE_XDR",
        account: "GABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF12",
      })
      .expect(401)
      .expect((res: any) => {
        expect(res.body.error).toContain("Invalid signature");
      });
  });

  it("returns 401 for an expired challenge", async () => {
    const { WebAuth } = jest.requireMock("@stellar/stellar-sdk");
    WebAuth.readChallengeTx.mockImplementation(() => {
      throw new Error("Challenge expired");
    });

    await request(app)
      .post("/auth")
      .send({
        transaction: "CHALLENGE_XDR",
        account: "GABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF12",
      })
      .expect(401)
      .expect((res: any) => {
        expect(res.body.error).toContain("expired");
      });
  });

  it("returns 401 for a wrong network passphrase", async () => {
    const { WebAuth } = jest.requireMock("@stellar/stellar-sdk");
    WebAuth.readChallengeTx.mockImplementation(() => {
      throw new Error("Wrong network passphrase");
    });

    await request(app)
      .post("/auth")
      .send({
        transaction: "CHALLENGE_XDR",
        account: "GABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF12",
      })
      .expect(401)
      .expect((res: any) => {
        expect(res.body.error).toContain("network");
      });
  });

  it("returns 401 for a replayed challenge", async () => {
    const { WebAuth } = jest.requireMock("@stellar/stellar-sdk");
    WebAuth.readChallengeTx.mockImplementation(() => {
      throw new Error("Challenge has already been used");
    });

    await request(app)
      .post("/auth")
      .send({
        transaction: "CHALLENGE_XDR",
        account: "GABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF12",
      })
      .expect(401)
      .expect((res: any) => {
        expect(res.body.error).toContain("already been used");
      });
  });
});

describe("Role derivation", () => {
  it("returns player for a registered player", async () => {
    mockedPrisma.player.findUnique.mockResolvedValue({ wallet: "GPLAYER123" });

    const role = await deriveRole("GPLAYER123");
    expect(role).toBe("player");
  });

  it("returns scout for a registered scout", async () => {
    mockedPrisma.scout.findUnique.mockResolvedValue({ wallet: "GSCOUT123" });

    const role = await deriveRole("GSCOUT123");
    expect(role).toBe("scout");
  });

  it("returns validator for an active validator", async () => {
    mockedPrisma.validator.findUnique.mockResolvedValue({
      wallet: "GVALID123",
      active: true,
    });

    const role = await deriveRole("GVALID123");
    expect(role).toBe("validator");
  });

  it("returns unknown for an inactive validator (falls through to player/scout/unknown)", async () => {
    mockedPrisma.validator.findUnique.mockResolvedValue({
      wallet: "GVALID123",
      active: false,
    });
    mockedPrisma.player.findUnique.mockResolvedValue(null);

    const role = await deriveRole("GVALID123");
    expect(role).toBe("unknown");
  });

  it("returns admin for a registered admin", async () => {
    mockedPrisma.admin.findUnique.mockResolvedValue({ address: "GADMIN123" });

    const role = await deriveRole("GADMIN123");
    expect(role).toBe("admin");
  });

  it("returns admin for the ADMIN_ADDRESS env var match", async () => {
    const originalAdmin = process.env.ADMIN_ADDRESS;
    process.env.ADMIN_ADDRESS = "GENVADMIN1234567890ABCDEF1234567890ABCDEF12345678";

    const role = await deriveRole("GENVADMIN1234567890ABCDEF1234567890ABCDEF12345678");
    expect(role).toBe("admin");

    if (originalAdmin !== undefined) {
      process.env.ADMIN_ADDRESS = originalAdmin;
    } else {
      delete process.env.ADMIN_ADDRESS;
    }
  });

  it("returns unknown for an unrecognised account", async () => {
    mockedPrisma.admin.findUnique.mockResolvedValue(null);
    mockedPrisma.validator.findUnique.mockResolvedValue(null);
    mockedPrisma.scout.findUnique.mockResolvedValue(null);
    mockedPrisma.player.findUnique.mockResolvedValue(null);

    const role = await deriveRole("GUNKNOWN123");
    expect(role).toBe("unknown");
  });

  it("prefers admin over other roles", async () => {
    mockedPrisma.admin.findUnique.mockResolvedValue({ address: "GADMIN123" });
    mockedPrisma.validator.findUnique.mockResolvedValue({
      wallet: "GADMIN123",
      active: true,
    });

    const role = await deriveRole("GADMIN123");
    expect(role).toBe("admin");
  });

  it("prefers validator over scout and player", async () => {
    mockedPrisma.validator.findUnique.mockResolvedValue({
      wallet: "GVALID123",
      active: true,
    });
    mockedPrisma.scout.findUnique.mockResolvedValue({ wallet: "GVALID123" });
    mockedPrisma.player.findUnique.mockResolvedValue({ wallet: "GVALID123" });

    const role = await deriveRole("GVALID123");
    expect(role).toBe("validator");
  });

  it("prefers scout over player", async () => {
    mockedPrisma.scout.findUnique.mockResolvedValue({ wallet: "GSCOUT123" });
    mockedPrisma.player.findUnique.mockResolvedValue({ wallet: "GSCOUT123" });

    const role = await deriveRole("GSCOUT123");
    expect(role).toBe("scout");
  });
});

/**
 * Role derivation for SEP-10 sessions (`deriveRole`).
 *
 * The HTTP challenge/verify/refresh flow is covered by `sep10Session.test.ts`
 * and `authEndpoint.test.ts`; this suite pins the role priority order
 * admin > validator > scout > player > unknown.
 */
jest.mock("../src/db", () => ({
  prisma: {
    admin: { findUnique: jest.fn() },
    validator: { findUnique: jest.fn() },
    scout: { findUnique: jest.fn() },
    player: { findUnique: jest.fn() },
  },
}));

import { prisma } from "../src/db";
import { config } from "../src/config";
import { deriveRole } from "../src/services/sep10";

const db = prisma as unknown as Record<string, { findUnique: jest.Mock }>;

beforeEach(() => {
  jest.resetAllMocks();
  for (const table of ["admin", "validator", "scout", "player"]) {
    db[table].findUnique.mockResolvedValue(null);
  }
});

describe("deriveRole", () => {
  it("returns player for a registered player", async () => {
    db.player.findUnique.mockResolvedValue({ wallet: "GPLAYER123" });
    expect(await deriveRole("GPLAYER123")).toBe("player");
  });

  it("returns scout for a registered scout", async () => {
    db.scout.findUnique.mockResolvedValue({ wallet: "GSCOUT123" });
    expect(await deriveRole("GSCOUT123")).toBe("scout");
  });

  it("returns validator for an active validator", async () => {
    db.validator.findUnique.mockResolvedValue({ wallet: "GVALID123", active: true });
    expect(await deriveRole("GVALID123")).toBe("validator");
  });

  it("falls through to unknown for an inactive validator", async () => {
    db.validator.findUnique.mockResolvedValue({ wallet: "GVALID123", active: false });
    expect(await deriveRole("GVALID123")).toBe("unknown");
  });

  it("returns admin for a registered admin", async () => {
    db.admin.findUnique.mockResolvedValue({ address: "GADMIN123" });
    expect(await deriveRole("GADMIN123")).toBe("admin");
  });

  it("returns admin for the configured ADMIN_ADDRESS", async () => {
    const original = config.adminAddress;
    config.adminAddress = "GENVADMIN";
    try {
      expect(await deriveRole("GENVADMIN")).toBe("admin");
      expect(db.admin.findUnique).not.toHaveBeenCalled();
    } finally {
      config.adminAddress = original;
    }
  });

  it("returns unknown for an unrecognised account", async () => {
    expect(await deriveRole("GUNKNOWN123")).toBe("unknown");
  });

  it("prefers admin over other roles", async () => {
    db.admin.findUnique.mockResolvedValue({ address: "GADMIN123" });
    db.validator.findUnique.mockResolvedValue({ wallet: "GADMIN123", active: true });
    expect(await deriveRole("GADMIN123")).toBe("admin");
  });

  it("prefers validator over scout and player", async () => {
    db.validator.findUnique.mockResolvedValue({ wallet: "GVALID123", active: true });
    db.scout.findUnique.mockResolvedValue({ wallet: "GVALID123" });
    db.player.findUnique.mockResolvedValue({ wallet: "GVALID123" });
    expect(await deriveRole("GVALID123")).toBe("validator");
  });

  it("prefers scout over player", async () => {
    db.scout.findUnique.mockResolvedValue({ wallet: "GSCOUT123" });
    db.player.findUnique.mockResolvedValue({ wallet: "GSCOUT123" });
    expect(await deriveRole("GSCOUT123")).toBe("scout");
  });
});

import request from "supertest";
import { app } from "../src/app";

jest.mock("../src/db", () => ({
  prisma: {
    scoutSubscription: {
      findUnique: jest.fn(),
    },
    player: {
      findMany: jest.fn(),
      findUnique: jest.fn(),
    },
  },
}));

import { prisma } from "../src/db";

describe("GET /api/players", () => {
  beforeEach(() => {
    jest.spyOn(console, "error").mockImplementation(() => {});
  });
  afterEach(() => {
    jest.restoreAllMocks();
    jest.clearAllMocks();
  });

  const mockScoutWallet = "GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N";

  it("returns 401 without authentication", async () => {
    const res = await request(app).get("/api/players");
    expect(res.status).toBe(401);
  });

  it("returns 403 for non-scout role", async () => {
    // Mock auth middleware to set role to "player"
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "player", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => {
        if (req.user?.role !== "scout" && req.user?.role !== "admin") {
          return res.status(403).json({ error: "Forbidden - insufficient role" });
        }
        next();
      }),
    }));

    const res = await request(app).get("/api/players");
    expect(res.status).toBe(403);
  });

  it("returns 400 for invalid region code", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => next()),
    }));

    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Pro",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    const res = await request(app).get("/api/players").query({ region: "INVALID" });
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid_region" });
  });

  it("returns 400 for invalid position code", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => next()),
    }));

    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Pro",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    const res = await request(app).get("/api/players").query({ position: "INVALID" });
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid_position" });
  });

  it("returns 400 for invalid minLevel", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => next()),
    }));

    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Pro",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    const res = await request(app).get("/api/players").query({ minLevel: 5 });
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid_min_level" });
  });

  it("returns paginated players for authenticated scout", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => next()),
    }));

    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Pro",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    const mockPlayers = [
      {
        playerId: BigInt(1),
        wallet: "GABC...",
        age: 20,
        position: "ST",
        region: "NG",
        nationality: "NG",
        level: 2,
        registeredAt: BigInt(1000000),
        updatedAt: BigInt(1000000),
        deactivated: false,
      },
      {
        playerId: BigInt(2),
        wallet: "GDEF...",
        age: 21,
        position: "FW",
        region: "GH",
        nationality: "GH",
        level: 1,
        registeredAt: BigInt(1000000),
        updatedAt: BigInt(1000000),
        deactivated: false,
      },
    ];

    (prisma.player.findMany as jest.Mock).mockResolvedValue(mockPlayers);

    const res = await request(app).get("/api/players");
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty("players");
    expect(res.body).toHaveProperty("cursor");
    expect(res.body).toHaveProperty("hasMore");
    expect(Array.isArray(res.body.players)).toBe(true);
  });

  it("applies tier visibility - wallet omitted for under-tier scouts", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => next()),
    }));

    // Basic tier scout - can only see Level 1+
    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Basic",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    const mockPlayers = [
      {
        playerId: BigInt(1),
        wallet: "GABC...",
        age: 20,
        position: "ST",
        region: "NG",
        nationality: "NG",
        level: 1, // Basic can see this
        registeredAt: BigInt(1000000),
        updatedAt: BigInt(1000000),
        deactivated: false,
      },
    ];

    (prisma.player.findMany as jest.Mock).mockResolvedValue(mockPlayers);

    const res = await request(app).get("/api/players");
    expect(res.status).toBe(200);
    expect(res.body.players[0]).toHaveProperty("wallet"); // Basic can see Level 1 wallet
  });

  it("excludes deactivated players", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => next()),
    }));

    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Pro",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    const mockPlayers = [
      {
        playerId: BigInt(1),
        wallet: "GABC...",
        age: 20,
        position: "ST",
        region: "NG",
        nationality: "NG",
        level: 2,
        registeredAt: BigInt(1000000),
        updatedAt: BigInt(1000000),
        deactivated: false,
      },
    ];

    (prisma.player.findMany as jest.Mock).mockResolvedValue(mockPlayers);

    const res = await request(app).get("/api/players");
    expect(res.status).toBe(200);
    // The WHERE clause should include deactivated: false
    expect(prisma.player.findMany).toHaveBeenCalledWith(
      expect.objectContaining({
        where: expect.objectContaining({
          deactivated: false,
        }),
      })
    );
  });

  it("respects limit parameter", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => next()),
    }));

    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Pro",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    (prisma.player.findMany as jest.Mock).mockResolvedValue([]);

    const res = await request(app).get("/api/players").query({ limit: 10 });
    expect(res.status).toBe(200);
    expect(prisma.player.findMany).toHaveBeenCalledWith(
      expect.objectContaining({
        take: 11, // limit + 1 for hasMore check
      })
    );
  });

  it("caps limit at 50", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
      requireRole: jest.fn(() => (req: any, res: any, next: any) => next()),
    }));

    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Pro",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    (prisma.player.findMany as jest.Mock).mockResolvedValue([]);

    const res = await request(app).get("/api/players").query({ limit: 100 });
    expect(res.status).toBe(200);
    expect(prisma.player.findMany).toHaveBeenCalledWith(
      expect.objectContaining({
        take: 51, // 50 + 1 for hasMore check
      })
    );
  });
});

describe("GET /api/players/:playerId", () => {
  beforeEach(() => {
    jest.spyOn(console, "error").mockImplementation(() => {});
  });
  afterEach(() => {
    jest.restoreAllMocks();
    jest.clearAllMocks();
  });

  const mockScoutWallet = "GD2DO5BIZZDWS5URPBXFHFZJR3VFXGGAFSRSAGNJA5AWVKXE53LRZM5N";
  const mockPlayerWallet = "GABCDEFGHIJKLMNOQRSTUVWXYZ123456789";

  it("returns 401 without authentication", async () => {
    const res = await request(app).get("/api/players/1");
    expect(res.status).toBe(401);
  });

  it("returns 400 for invalid player ID", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
    }));

    const res = await request(app).get("/api/players/invalid");
    expect(res.status).toBe(400);
    expect(res.body).toEqual({ error: "invalid_player_id" });
  });

  it("returns 404 for non-existent player", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
    }));

    (prisma.player.findUnique as jest.Mock).mockResolvedValue(null);

    const res = await request(app).get("/api/players/999");
    expect(res.status).toBe(404);
    expect(res.body).toEqual({ error: "player_not_found" });
  });

  it("returns 404 for deactivated player", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
    }));

    (prisma.player.findUnique as jest.Mock).mockResolvedValue({
      playerId: BigInt(1),
      wallet: mockPlayerWallet,
      age: 20,
      position: "ST",
      region: "NG",
      nationality: "NG",
      level: 2,
      registeredAt: BigInt(1000000),
      updatedAt: BigInt(1000000),
      deactivated: true,
    });

    const res = await request(app).get("/api/players/1");
    expect(res.status).toBe(404);
    expect(res.body).toEqual({ error: "player_not_found" });
  });

  it("returns full data for the player themselves", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockPlayerWallet, role: "player", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
    }));

    const mockPlayer = {
      playerId: BigInt(1),
      wallet: mockPlayerWallet,
      age: 20,
      position: "ST",
      region: "NG",
      nationality: "NG",
      level: 2,
      registeredAt: BigInt(1000000),
      updatedAt: BigInt(1000000),
      deactivated: false,
      ipfsHashes: ["QmHash1", "QmHash2"],
    };

    (prisma.player.findUnique as jest.Mock).mockResolvedValue(mockPlayer);

    const res = await request(app).get("/api/players/1");
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty("wallet");
    expect(res.body).toHaveProperty("ipfsHashes");
    expect(res.body.wallet).toBe(mockPlayerWallet);
  });

  it("applies tier visibility for scouts", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: mockScoutWallet, role: "scout", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
    }));

    // Basic tier scout
    (prisma.scoutSubscription.findUnique as jest.Mock).mockResolvedValue({
      scout: mockScoutWallet,
      tier: "Basic",
      subscribedAt: Date.now(),
      expiresAt: Date.now() + 1000000,
    });

    const mockPlayer = {
      playerId: BigInt(1),
      wallet: mockPlayerWallet,
      age: 20,
      position: "ST",
      region: "NG",
      nationality: "NG",
      level: 2, // Basic cannot see Level 2
      registeredAt: BigInt(1000000),
      updatedAt: BigInt(1000000),
      deactivated: false,
      ipfsHashes: ["QmHash1"],
    };

    (prisma.player.findUnique as jest.Mock).mockResolvedValue(mockPlayer);

    const res = await request(app).get("/api/players/1");
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty("wallet");
    expect(res.body.wallet).toBeUndefined(); // Basic cannot see Level 2 wallet
  });

  it("returns full data for admins", async () => {
    const mockRequireAuth = (req: any, res: any, next: any) => {
      req.user = { sub: "GADMIN...", role: "admin", jti: "test-jti" };
      next();
    };
    jest.mock("../src/middleware/auth", () => ({
      requireAuth: mockRequireAuth,
    }));

    const mockPlayer = {
      playerId: BigInt(1),
      wallet: mockPlayerWallet,
      age: 20,
      position: "ST",
      region: "NG",
      nationality: "NG",
      level: 3,
      registeredAt: BigInt(1000000),
      updatedAt: BigInt(1000000),
      deactivated: false,
      ipfsHashes: ["QmHash1"],
    };

    (prisma.player.findUnique as jest.Mock).mockResolvedValue(mockPlayer);

    const res = await request(app).get("/api/players/1");
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty("wallet");
    expect(res.body).toHaveProperty("ipfsHashes");
    expect(res.body.wallet).toBe(mockPlayerWallet);
  });
});

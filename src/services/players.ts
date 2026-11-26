import { prisma } from "../db";

/**
 * ProgressLevel enum values matching the on-chain definition
 */
export enum ProgressLevel {
  Unverified = 0,
  VerifiedIdentity = 1,
  PerformanceMilestones = 2,
  EliteTier = 3,
}

/**
 * Subscription tier enum matching the on-chain definition
 */
export enum SubscriptionTier {
  None = "None",
  Basic = "Basic",
  Pro = "Pro",
  Elite = "Elite",
}

/**
 * Minimum tier required to view each ProgressLevel
 * Derived from the cross-doc consistency check (scripts/check-cross-doc-consistency.sh)
 */
const LEVEL_TO_MIN_TIER: Record<ProgressLevel, SubscriptionTier> = {
  [ProgressLevel.Unverified]: SubscriptionTier.None,
  [ProgressLevel.VerifiedIdentity]: SubscriptionTier.Basic,
  [ProgressLevel.PerformanceMilestones]: SubscriptionTier.Pro,
  [ProgressLevel.EliteTier]: SubscriptionTier.Elite,
};

/**
 * Tier rank for comparison (higher rank = more access)
 */
const TIER_RANK: Record<SubscriptionTier, number> = {
  [SubscriptionTier.None]: 0,
  [SubscriptionTier.Basic]: 1,
  [SubscriptionTier.Pro]: 2,
  [SubscriptionTier.Elite]: 3,
};

/**
 * Canonical position codes matching on-chain validation in registration contract
 */
const VALID_POSITIONS = new Set([
  "GK", // Goalkeeper
  "CB", // Center Back
  "LB", // Left Back
  "RB", // Right Back
  "CDM", // Central Defensive Midfielder
  "CM", // Central Midfielder
  "CAM", // Central Attacking Midfielder
  "LW", // Left Winger
  "RW", // Right Winger
  "ST", // Striker
  "CF", // Center Forward
]);

/**
 * ISO 3166-1 alpha-2 country codes (subset for football regions)
 * This is a practical subset; in production this should be expanded
 */
const VALID_REGIONS = new Set([
  "NG", // Nigeria
  "GH", // Ghana
  "KE", // Kenya
  "ZA", // South Africa
  "CI", // Ivory Coast
  "SN", // Senegal
  "CM", // Cameroon
  "MA", // Morocco
  "TN", // Tunisia
  "EG", // Egypt
  "BR", // Brazil
  "AR", // Argentina
  "UY", // Uruguay
  "CO", // Colombia
  "MX", // Mexico
  "US", // United States
  "DE", // Germany
  "FR", // France
  "ES", // Spain
  "IT", // Italy
  "GB", // United Kingdom
  "PT", // Portugal
  "NL", // Netherlands
  "BE", // Belgium
  "JP", // Japan
  "KR", // South Korea
  "AU", // Australia
]);

/**
 * Player summary for API responses
 * Wallet and IPFS hashes are omitted for under-tier scouts
 */
export interface PlayerSummary {
  playerId: bigint;
  age: number;
  position: string;
  region: string;
  nationality: string;
  level: number;
  registeredAt: bigint;
  updatedAt: bigint;
  wallet?: string; // Only included if scout has sufficient tier
  ipfsHashes?: string[]; // Only included if scout has sufficient tier
}

/**
 * Paginated player list response
 */
export interface PaginatedPlayers {
  players: PlayerSummary[];
  cursor: string | null;
  hasMore: boolean;
}

/**
 * Opaque cursor for keyset pagination
 * Encoded as: level:playerId (e.g., "2:12345")
 */
function encodeCursor(level: number, playerId: bigint): string {
  return `${level}:${playerId.toString()}`;
}

/**
 * Decode opaque cursor
 */
function decodeCursor(cursor: string): { level: number; playerId: bigint } | null {
  const parts = cursor.split(":");
  if (parts.length !== 2) return null;
  const level = parseInt(parts[0], 10);
  const playerId = BigInt(parts[1]);
  if (isNaN(level) || isNaN(Number(playerId))) return null;
  return { level, playerId };
}

/**
 * Check if a scout's tier allows viewing a player at a given level
 */
function canViewLevel(scoutTier: SubscriptionTier, playerLevel: number): boolean {
  const minTier = LEVEL_TO_MIN_TIER[playerLevel as ProgressLevel];
  return TIER_RANK[scoutTier] >= TIER_RANK[minTier];
}

/**
 * Get a scout's current subscription tier from the database
 * Returns None if no subscription exists or subscription is expired
 */
async function getScoutTier(scoutWallet: string): Promise<SubscriptionTier> {
  const now = Math.floor(Date.now() / 1000);
  const subscription = await prisma.scoutSubscription.findUnique({
    where: { scout: scoutWallet },
  });

  if (!subscription) {
    return SubscriptionTier.None;
  }

  // Check if subscription is expired
  if (subscription.expiresAt < now) {
    return SubscriptionTier.None;
  }

  return subscription.tier as SubscriptionTier;
}

/**
 * Query players with filters, tier visibility, and keyset pagination
 *
 * @param scoutWallet - The authenticated scout's wallet address
 * @param filters - Optional filters (region, position, minLevel)
 * @param cursor - Opaque pagination cursor
 * @param limit - Maximum number of results (capped at 50)
 */
export async function listPlayers(
  scoutWallet: string,
  filters: {
    region?: string;
    position?: string;
    minLevel?: number;
  } = {},
  cursor: string | null = null,
  limit: number = 20
): Promise<PaginatedPlayers> {
  // Validate and cap limit
  const effectiveLimit = Math.min(Math.max(1, limit), 50);

  // Validate region if provided
  if (filters.region && !VALID_REGIONS.has(filters.region)) {
    throw new Error("Invalid region code");
  }

  // Validate position if provided
  if (filters.position && !VALID_POSITIONS.has(filters.position)) {
    throw new Error("Invalid position code");
  }

  // Validate minLevel if provided
  if (filters.minLevel !== undefined) {
    if (filters.minLevel < 0 || filters.minLevel > 3) {
      throw new Error("Invalid minLevel");
    }
  }

  // Get scout's tier
  const scoutTier = await getScoutTier(scoutWallet);

  // Build WHERE clause
  const where: any = {
    deactivated: false, // Exclude deactivated players
  };

  if (filters.region) {
    where.region = filters.region;
  }

  if (filters.position) {
    where.position = filters.position;
  }

  if (filters.minLevel !== undefined) {
    where.level = { gte: filters.minLevel };
  }

  // Build cursor-based pagination
  let cursorWhere: any = {};
  if (cursor) {
    const decoded = decodeCursor(cursor);
    if (!decoded) {
      throw new Error("Invalid cursor");
    }
    // Keyset pagination: (level DESC, player_id ASC)
    // We want rows where level < cursor.level OR (level = cursor.level AND player_id > cursor.player_id)
    cursorWhere = {
      OR: [
        { level: { lt: decoded.level } },
        { level: decoded.level, playerId: { gt: decoded.playerId } },
      ],
    };
  }

  // Query with tier visibility filter
  // We need to filter by level based on scout's tier
  const minVisibleLevel = Object.entries(LEVEL_TO_MIN_TIER)
    .filter(([_, minTier]) => TIER_RANK[minTier as SubscriptionTier] <= TIER_RANK[scoutTier])
    .map(([level]) => parseInt(level, 10));

  if (minVisibleLevel.length > 0) {
    where.level = { ...where.level, in: minVisibleLevel };
  } else {
    // Scout has no access to any level
    return { players: [], cursor: null, hasMore: false };
  }

  // Execute query
  const players = await prisma.player.findMany({
    where: { ...where, ...cursorWhere },
    orderBy: [
      { level: "desc" as const },
      { playerId: "asc" as const },
    ],
    take: effectiveLimit + 1, // Fetch one extra to determine if there are more
  });

  // Determine if there are more results
  const hasMore = players.length > effectiveLimit;
  const results = hasMore ? players.slice(0, effectiveLimit) : players;

  // Build next cursor
  let nextCursor: string | null = null;
  if (hasMore && results.length > 0) {
    const last = results[results.length - 1];
    nextCursor = encodeCursor(last.level, last.playerId);
  }

  // Apply tier visibility to each player (omit wallet/ipfs for under-tier)
  const playerSummaries: PlayerSummary[] = results.map((player) => {
    const canViewFull = canViewLevel(scoutTier, player.level);
    return {
      playerId: player.playerId,
      age: player.age,
      position: player.position,
      region: player.region,
      nationality: player.nationality,
      level: player.level,
      registeredAt: player.registeredAt,
      updatedAt: player.updatedAt,
      wallet: canViewFull ? player.wallet : undefined,
      ipfsHashes: canViewFull ? player.ipfsHashes : undefined,
    };
  });

  return {
    players: playerSummaries,
    cursor: nextCursor,
    hasMore,
  };
}

/**
 * Get a single player by ID with tier visibility applied
 * Accessible to the player themselves or scouts with sufficient tier
 */
export async function getPlayer(
  playerId: bigint,
  requesterWallet: string,
  requesterRole: string
): Promise<PlayerSummary | null> {
  const player = await prisma.player.findUnique({
    where: { playerId },
  });

  if (!player || player.deactivated) {
    return null;
  }

  // If requester is the player themselves, return full data
  if (player.wallet.toLowerCase() === requesterWallet.toLowerCase()) {
    return {
      playerId: player.playerId,
      age: player.age,
      position: player.position,
      region: player.region,
      nationality: player.nationality,
      level: player.level,
      registeredAt: player.registeredAt,
      updatedAt: player.updatedAt,
      wallet: player.wallet,
      ipfsHashes: player.ipfsHashes,
    };
  }

  // If requester is a scout, apply tier visibility
  if (requesterRole === "scout") {
    const scoutTier = await getScoutTier(requesterWallet);
    const canViewFull = canViewLevel(scoutTier, player.level);

    return {
      playerId: player.playerId,
      age: player.age,
      position: player.position,
      region: player.region,
      nationality: player.nationality,
      level: player.level,
      registeredAt: player.registeredAt,
      updatedAt: player.updatedAt,
      wallet: canViewFull ? player.wallet : undefined,
      ipfsHashes: canViewFull ? player.ipfsHashes : undefined,
    };
  }

  // Admin can see everything
  if (requesterRole === "admin") {
    return {
      playerId: player.playerId,
      age: player.age,
      position: player.position,
      region: player.region,
      nationality: player.nationality,
      level: player.level,
      registeredAt: player.registeredAt,
      updatedAt: player.updatedAt,
      wallet: player.wallet,
      ipfsHashes: player.ipfsHashes,
    };
  }

  return null;
}

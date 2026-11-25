import { prisma } from "../db";
import { config } from "../config";

/**
 * Maximum access token TTL in seconds (15 minutes).
 * Used as a fallback when expires_at is NULL in revoked_tokens.
 */
const MAX_ACCESS_TOKEN_TTL_SECONDS = 15 * 60;

/**
 * Cleanup policy intervals (in days/seconds)
 */
const CHALLENGE_CLEANUP_DAYS = 1;
const REFRESH_TOKEN_CLEANUP_DAYS = 1;
const REFRESH_TOKEN_REVOKED_CLEANUP_DAYS = 1;

/**
 * Delete expired SEP-10 challenges that are older than the cleanup window.
 */
async function cleanupSep10Challenges(): Promise<number> {
  const cutoff = new Date();
  cutoff.setDate(cutoff.getDate() - CHALLENGE_CLEANUP_DAYS);

  const result = await prisma.sep10Challenge.deleteMany({
    where: {
      expiresAt: { lt: cutoff },
    },
  });

  return result.count;
}

/**
 * Delete expired refresh tokens and revoked refresh tokens older than the cleanup window.
 */
async function cleanupRefreshTokens(): Promise<number> {
  const cutoff = new Date();
  cutoff.setDate(cutoff.getDate() - REFRESH_TOKEN_CLEANUP_DAYS);

  // Delete tokens that expired or were revoked more than the cleanup window ago
  const result = await prisma.refreshToken.deleteMany({
    where: {
      OR: [
        { expiresAt: { lt: cutoff } },
        { revokedAt: { lt: cutoff } },
      ],
    },
  });

  return result.count;
}

/**
 * Delete revoked tokens where the expiry has passed.
 * Falls back to created_at + max_ttl if expires_at is NULL.
 */
async function cleanupRevokedTokens(): Promise<number> {
  const now = new Date();
  const maxTtlMs = MAX_ACCESS_TOKEN_TTL_SECONDS * 1000;

  // Delete tokens where expires_at < now, OR (expires_at is NULL and created_at + maxTtl < now)
  const result = await prisma.$executeRaw`
    DELETE FROM revoked_tokens
    WHERE expires_at < ${now}
       OR (expires_at IS NULL AND created_at + INTERVAL '1 second' * ${MAX_ACCESS_TOKEN_TTL_SECONDS} < ${now})
  `;

  return result as number;
}

/**
 * Run all cleanup jobs and return summary of deleted rows.
 */
export async function runCleanup(): Promise<{
  sep10Challenges: number;
  refreshTokens: number;
  revokedTokens: number;
}> {
  const sep10Challenges = await cleanupSep10Challenges();
  const refreshTokens = await cleanupRefreshTokens();
  const revokedTokens = await cleanupRevokedTokens();

  return {
    sep10Challenges,
    refreshTokens,
    revokedTokens,
  };
}

/**
 * Start the cleanup job scheduler.
 * Runs cleanup at the configured interval.
 * Does NOT run immediately; call runCleanup() once before starting if needed.
 *
 * @param intervalSeconds Interval in seconds between runs. Uses config.cleanupIntervalSeconds if not provided.
 */
export function startCleanupJob(intervalSeconds?: number) {
  const interval = intervalSeconds ?? config.cleanupIntervalSeconds;

  if (interval <= 0) {
    console.log("Cleanup job disabled (CLEANUP_INTERVAL_SECONDS <= 0)");
    return;
  }

  console.log(`Starting cleanup job with interval ${interval}s`);

  // Run immediately on startup
  runCleanup()
    .then((result) => {
      console.log(`Initial cleanup: ${JSON.stringify(result)}`);
    })
    .catch((error) => {
      console.error("Cleanup job failed on startup:", error);
    });

  // Then run on interval
  const intervalId = setInterval(async () => {
    try {
      const result = await runCleanup();
      console.log(`Scheduled cleanup: ${JSON.stringify(result)}`);
    } catch (error) {
      console.error("Cleanup job failed:", error);
    }
  }, interval * 1000);

  // Handle graceful shutdown
  process.on("SIGTERM", () => {
    console.log("Received SIGTERM, stopping cleanup job");
    clearInterval(intervalId);
    process.exit(0);
  });

  process.on("SIGINT", () => {
    console.log("Received SIGINT, stopping cleanup job");
    clearInterval(intervalId);
    process.exit(0);
  });
}

import rateLimit, { RateLimitRequestHandler } from "express-rate-limit";
import { prisma } from "../db";
import { config } from "../config";

/**
 * Rate limiting configuration
 */

/**
 * Get client IP address, respecting reverse proxies.
 * Uses req.ip when app.set("trust proxy") is enabled, otherwise falls back to socket address.
 */
export function getClientIp(req: { ip: string; connection?: { remoteAddress?: string } }): string {
  return req.ip || (req.connection?.remoteAddress as string) || "unknown";
}

/**
 * Rate limit window in milliseconds (1 minute)
 */
const RATE_LIMIT_WINDOW_MS = 60 * 1000; // 1 minute

/**
 * Shared rate limit store using PostgreSQL.
 * Stores rate limit counters in a simple key-value format.
 * Note: For high-throughput scenarios, Redis is recommended for better performance.
 * This PostgreSQL implementation is suitable for single-instance or low-to-mid traffic deployments.
 */
const RATE_LIMIT_KEY_PREFIX = "ratelimit:";

/**
 * Generate a unique rate limit key for the given identifier (IP or account)
 */
function makeRateLimitKey(type: "ip" | "account", identifier: string): string {
  return `${RATE_LIMIT_KEY_PREFIX}${type}:${identifier}`;
}

/**
 * Initialize rate limiting middleware
 */
export function initRateLimiting(app: { set: (key: string, value: unknown) => void }) {
  // Configure trust proxy from environment variable
  // When behind a load balancer, set TRUST_PROXY=true to use X-Forwarded-For headers
  // When not behind a proxy, leave it unset or set to false to prevent IP spoofing
  const trustProxy = config.trustProxy !== undefined ? config.trustProxy : false;
  app.set("trust proxy", trustProxy);

  // Default window is 1 minute (60000 ms)
  const windowMs = 60 * 1000;

  return {
    // GET /auth: 10 requests per minute per IP, 5 per account
    getAuth: rateLimit({
      windowMs: windowMs,
      max: config.getAuthIpLimit, // IP limit
      keyGenerator: (req) => getClientIp(req),
      standardHeaders: true,
      legacyHeaders: false,
      handler: (req, res) => {
        res.status(429).json({
          error: "rate_limited",
          message: "Too many requests",
          retryAfter: Math.ceil(windowMs / 1000),
        });
      },
      skipSuccessfulRequests: false,
    }),

    // GET /auth with per-account tracking
    getAuthAccount: rateLimit({
      windowMs: windowMs,
      max: config.getAuthAccountLimit, // Per-account limit
      keyGenerator: (req) => {
        const account = req.query.account;
        if (typeof account === "string" && account.length > 0) {
          return makeRateLimitKey("account", account);
        }
        return getClientIp(req);
      },
      standardHeaders: true,
      legacyHeaders: false,
      handler: (req, res) => {
        res.status(429).json({
          error: "rate_limited",
          message: "Too many requests",
          retryAfter: Math.ceil(windowMs / 1000),
        });
      },
      skipSuccessfulRequests: false,
    }),

    // POST /auth: 20 requests per minute per IP
    postAuth: rateLimit({
      windowMs: windowMs,
      max: config.postAuthIpLimit,
      keyGenerator: (req) => getClientIp(req),
      standardHeaders: true,
      legacyHeaders: false,
      handler: (req, res) => {
        res.status(429).json({
          error: "rate_limited",
          message: "Too many requests",
          retryAfter: Math.ceil(windowMs / 1000),
        });
      },
      skipSuccessfulRequests: false,
    }),

    // POST /auth/refresh: 10 requests per minute per IP
    postAuthRefresh: rateLimit({
      windowMs: windowMs,
      max: config.postAuthRefreshIpLimit,
      keyGenerator: (req) => getClientIp(req),
      standardHeaders: true,
      legacyHeaders: false,
      handler: (req, res) => {
        res.status(429).json({
          error: "rate_limited",
          message: "Too many requests",
          retryAfter: Math.ceil(windowMs / 1000),
        });
      },
      skipSuccessfulRequests: false,
    }),

    // Admin endpoints: 10 requests per minute per IP
    admin: rateLimit({
      windowMs: windowMs,
      max: config.adminIpLimit,
      keyGenerator: (req) => getClientIp(req),
      standardHeaders: true,
      legacyHeaders: false,
      handler: (req, res) => {
        res.status(429).json({
          error: "rate_limited",
          message: "Too many requests",
          retryAfter: Math.ceil(windowMs / 1000),
        });
      },
      skipSuccessfulRequests: false,
    }),
  };
}

/**
 * Optional: Cap outstanding unconsumed challenges per account
 * Returns true if the challenge count is within the limit
 */
export async function checkChallengeLimit(account: string, maxChallenges: number = 5): Promise<boolean> {
  const result = await prisma.sep10Challenge.findMany({
    where: {
      account,
      consumedAt: null,
      expiresAt: { gt: new Date() },
    },
    select: { hash: true },
    orderBy: { expiresAt: "asc" },
    take: maxChallenges + 1,
  });

  if (result.length > maxChallenges) {
    // Delete the oldest unconsumed challenge to make room
    await prisma.sep10Challenge.deleteMany({
      where: { hash: result[0].hash },
    });
  }

  return result.length <= maxChallenges;
}


import client, { collectDefaultMetrics, register } from "prom-client";

// Collect default process metrics (CPU, memory, event loop lag, etc.)
collectDefaultMetrics({ register });

/**
 * HTTP request duration histogram with method and route labels.
 * Uses templates to avoid unbounded cardinality from account IDs.
 */
export const httpRequestDuration = new client.Histogram({
  name: "http_request_duration_seconds",
  help: "Duration of HTTP requests in seconds",
  labelNames: ["method", "route", "status"] as const,
  // Bucket boundaries: 5ms to 10s
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
});

/**
 * SEP-10 authentication challenges issued.
 */
export const authChallengesIssued = new client.Counter({
  name: "auth_challenges_issued_total",
  help: "Total number of SEP-10 challenges issued",
  labelNames: [] as const,
});

/**
 * SEP-10 authentication verifications with result codes.
 * Result codes are stable error codes (not raw error messages).
 */
export const authVerifications = new client.Counter({
  name: "auth_verifications_total",
  help: "Total number of SEP-10 authentication verifications by result",
  labelNames: ["result"] as const,
});

/**
 * Token refresh operations with result codes.
 */
export const authRefreshes = new client.Counter({
  name: "auth_refresh_total",
  help: "Total number of token refreshes by result",
  labelNames: ["result"] as const,
});

/**
 * Token revocations.
 */
export const tokensRevoked = new client.Counter({
  name: "tokens_revoked_total",
  help: "Total number of tokens revoked",
  labelNames: [] as const,
});

/**
 * Database query duration with model and action labels.
 * Uses a Prisma middleware to capture this data.
 */
export const dbQueryDuration = new client.Histogram({
  name: "db_query_duration_seconds",
  help: "Duration of database queries in seconds",
  labelNames: ["model", "action"] as const,
  // Bucket boundaries: 0.1ms to 1s
  buckets: [0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1],
});

/**
 * Generate a route label from a request URL, using templates for parameters.
 * Examples:
 *   /auth -> /auth
 *   /api/tokens/me -> /api/tokens/me
 *   /api/admin/tokens/revoke -> /api/admin/tokens/revoke
 */
export function getRouteLabel(path: string): string {
  // Remove query parameters
  const basePath = path.split("?")[0];

  // Template common patterns
  const templates: Array<[RegExp, string]> = [
    [/\b[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}\b/g, ":uuid"],
    [/\b[a-f0-9]{64}\b/g, ":hash"], // 64-char hex (e.g., transaction hashes)
    [/\b[a-f0-9]{56}\b/g, ":account"], // 56-char base58 (e.g., Stellar addresses)
    [/\b\d+\b/g, ":id"],
  ];

  let result = basePath;
  for (const [pattern, replacement] of templates) {
    result = result.replace(pattern, replacement);
  }

  return result;
}

// Export the metrics registry for the metrics endpoint
export { register };

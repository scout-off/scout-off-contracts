import { Request, Response, NextFunction } from "express";
import { logger } from "../logger";

/**
 * Check if the metrics endpoint request includes a valid token.
 * Supports both query parameter (?token=<value>) and Authorization header (Bearer <value>).
 */
export function isMetricsRequestAuthorized(req: Request, token?: string): boolean {
  if (!token) {
    // No token configured, metrics are public
    return true;
  }

  const queryToken = req.query.token;
  if (typeof queryToken === "string" && queryToken === token) {
    return true;
  }

  const authHeader = req.headers["authorization"];
  if (
    typeof authHeader === "string" &&
    authHeader.startsWith("Bearer ") &&
    authHeader.slice(7) === token
  ) {
    return true;
  }

  return false;
}

/**
 * Metrics authentication middleware.
 * Requires a valid token if METRICS_TOKEN is configured.
 */
export function metricsAuthMiddleware(req: Request, res: Response, next: NextFunction): void {
  const token = process.env.METRICS_TOKEN?.trim();

  if (!isMetricsRequestAuthorized(req, token)) {
    logger.warn({ path: req.path }, "GET /metrics: unauthorized");
    res.status(401).json({ error: "Unauthorized" });
    return;
  }

  next();
}

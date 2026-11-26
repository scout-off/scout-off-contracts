import pino, { Logger } from "pino";
import { Request, Response } from "express";
import { randomUUID } from "crypto";

const DEV_MODE = process.env.NODE_ENV !== "production";

const redactPaths = [
  "req.headers.authorization",
  "*.transaction",
  "*.refreshToken",
  "*.token",
];

const options: pino.LoggerOptions = {
  level: process.env.LOG_LEVEL || "info",
  redact: {
    paths: redactPaths,
    censor: "[REDACTED]",
  },
  transport: DEV_MODE
    ? {
        target: "pino-pretty",
        options: {
          colorize: true,
          translateTime: "SYS:standard",
          ignore: "pid,hostname",
        },
      }
    : undefined,
};

export const logger: Logger = pino(options);

/**
 * Generate a unique request ID. If the incoming request already has an
 * x-request-id header, reuse it for correlation.
 */
export function generateRequestId(req?: Request): string {
  const existing = req?.headers["x-request-id"];
  if (typeof existing === "string" && existing.length > 0) {
    return existing;
  }
  // Use a short, URL-safe ID (8 chars gives ~2^48 combinations)
  return randomUUID().split("-")[0];
}

/**
 * Attach a request ID to the response headers for client-side correlation.
 */
export function attachRequestIdToResponse(req: Request, res: Response): void {
  const requestId = (req as Request & { requestId?: string }).requestId;
  if (requestId) {
    res.setHeader("x-request-id", requestId);
  }
}

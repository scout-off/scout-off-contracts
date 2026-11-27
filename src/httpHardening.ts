import express, { Express, NextFunction, Request, Response } from "express";

/** Parse the comma-separated CORS_ORIGINS allow-list (read at request time). */
export function corsOrigins(): string[] {
  return (process.env.CORS_ORIGINS ?? "")
    .split(",")
    .map((o) => o.trim())
    .filter(Boolean);
}

export const JSON_BODY_LIMIT = process.env.JSON_BODY_LIMIT || "16kb";

function cors(req: Request, res: Response, next: NextFunction) {
  const origin = req.headers.origin;
  if (!origin) return next();
  if (!corsOrigins().includes(origin)) {
    return res.status(403).json({ error: "origin_not_allowed" });
  }
  res.setHeader("Access-Control-Allow-Origin", origin);
  res.setHeader("Vary", "Origin");
  if (req.method === "OPTIONS") {
    res.setHeader("Access-Control-Allow-Methods", "GET,POST,OPTIONS");
    res.setHeader(
      "Access-Control-Allow-Headers",
      req.headers["access-control-request-headers"] ?? "Content-Type, Authorization"
    );
    res.setHeader("Access-Control-Max-Age", "600");
    return res.status(204).end();
  }
  next();
}

function securityHeaders(_req: Request, res: Response, next: NextFunction) {
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.setHeader("X-Frame-Options", "DENY");
  res.setHeader("Referrer-Policy", "no-referrer");
  res.setHeader("Cross-Origin-Resource-Policy", "same-origin");
  res.setHeader("Content-Security-Policy", "default-src 'none'; frame-ancestors 'none'");
  res.setHeader("Strict-Transport-Security", "max-age=15552000; includeSubDomains");
  next();
}

function jsonErrorHandler(err: any, _req: Request, res: Response, next: NextFunction) {
  if (err?.type === "entity.parse.failed") {
    return res.status(400).json({ error: "invalid_json" });
  }
  if (err?.type === "entity.too.large") {
    return res.status(413).json({ error: "payload_too_large" });
  }
  next(err);
}

/** Install CORS, security headers and the size-limited JSON parser. */
export function applyHttpHardening(app: Express): void {
  app.disable("x-powered-by");
  app.use(securityHeaders);
  app.use(cors);
  app.use(express.json({ limit: JSON_BODY_LIMIT }));
  app.use(jsonErrorHandler);
}

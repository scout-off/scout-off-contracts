import express, { type Express, type NextFunction, type Request, type Response, type RequestHandler } from "express";
import jwt from "jsonwebtoken";
import { createEvidenceRouter, type EvidenceDatabase } from "./routes/evidence";
import type { EvidenceGrantChecker } from "./services/evidenceGrantChecker";

export interface EvidenceAppDependencies {
  database: EvidenceDatabase;
  grantChecker: EvidenceGrantChecker;
  masterKey: Buffer;
  /** HS256 secret used to verify SEP-10 session JWTs. */
  jwtSecret: string;
  /** Returns true when the token's `jti` has been revoked. */
  isTokenRevoked: (jti: string) => Promise<boolean>;
}

/**
 * Build an Express middleware that authenticates a `Bearer` SEP-10 JWT
 * (HS256, with `sub`, `role` and `jti` claims) and populates `req.user`.
 */
export function createAuthenticate(
  jwtSecret: string,
  isTokenRevoked: (jti: string) => Promise<boolean>
): RequestHandler {
  return async (req: Request, res: Response, next: NextFunction) => {
    const header = req.headers.authorization?.trim();
    const match = header ? /^Bearer\s+(\S+)$/i.exec(header) : null;
    if (!match) {
      res.status(401).json({ error: "Unauthorized" });
      return;
    }

    let payload: jwt.JwtPayload;
    try {
      const verified = jwt.verify(match[1], jwtSecret, { algorithms: ["HS256"] });
      if (typeof verified === "string") {
        res.status(401).json({ error: "Unauthorized" });
        return;
      }
      payload = verified;
    } catch {
      res.status(401).json({ error: "Unauthorized" });
      return;
    }

    if (
      typeof payload.sub !== "string" ||
      typeof payload.role !== "string" ||
      typeof payload.jti !== "string" ||
      payload.jti.length === 0
    ) {
      res.status(401).json({ error: "Unauthorized" });
      return;
    }

    try {
      if (await isTokenRevoked(payload.jti)) {
        res.status(401).json({ error: "Unauthorized" });
        return;
      }
    } catch (error) {
      next(error);
      return;
    }

    req.user = { sub: payload.sub, role: payload.role, jti: payload.jti };
    next();
  };
}

/**
 * Standalone app for the evidence key-wrapping API (#1775). Dependencies are
 * injected so the API can be mounted by a server process that owns the
 * database pool, master key and on-chain grant checker.
 */
export function createApp(dependencies: EvidenceAppDependencies): Express {
  const app = express();
  app.use(express.json());
  app.use(
    "/api/evidence",
    createEvidenceRouter({
      database: dependencies.database,
      grantChecker: dependencies.grantChecker,
      masterKey: dependencies.masterKey,
      authenticate: createAuthenticate(dependencies.jwtSecret, dependencies.isTokenRevoked),
    })
  );
  app.use((_error: unknown, _req: Request, res: Response, _next: NextFunction) => {
    res.status(500).json({ error: "Internal server error" });
  });
  return app;
}

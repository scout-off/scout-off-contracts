import jwt, { type JwtPayload } from "jsonwebtoken";
import type { RequestHandler } from "express";

export interface AuthenticatedUser {
  sub: string;
  role: string;
  jti: string;
}

declare global {
  namespace Express {
    interface Request {
      user?: AuthenticatedUser;
    }
  }
}

export function createRequireAuth(
  secret: string,
  isTokenRevoked: (jti: string) => Promise<boolean>
): RequestHandler {
  if (!secret) {
    throw new Error("JWT_SECRET is required");
  }

  return async (req, res, next) => {
    const authHeader = req.get("authorization");
    if (!authHeader?.startsWith("Bearer ")) {
      res.status(401).json({ error: "Unauthorized" });
      return;
    }

    let payload: JwtPayload;
    try {
      const verified = jwt.verify(authHeader.slice(7), secret, {
        algorithms: ["HS256"],
      });
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

export function requireRole(roles: readonly string[]): RequestHandler {
  return (req, res, next) => {
    if (!req.user) {
      res.status(401).json({ error: "Unauthorized" });
      return;
    }
    if (!roles.includes(req.user.role)) {
      res.status(403).json({ error: "Forbidden" });
      return;
    }
    next();
  };
}

import jwt from "jsonwebtoken";
import { Request, Response, NextFunction } from "express";
import { prisma } from "../db";
import { config } from "../config";

// Extend the Express Request type so downstream handlers can access req.user
// without casting.
declare global {
  namespace Express {
    interface Request {
      user?: {
        sub: string;
        role: string;
        jti: string;
      };
    }
  }
}

export async function isTokenRevoked(jti: string): Promise<boolean> {
  if (!jti) {
    return false;
  }

  const existingRevocation = await prisma.revokedToken.findFirst({
    where: { jti },
  });

  return !!existingRevocation;
}

// RFC 6750: the auth scheme is case-insensitive; tolerate extra whitespace
// before the token. The token must be a compact JWT (three segments).
const BEARER_JWT_PATTERN =
  /^Bearer\s+([A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]*)$/i;

export async function requireAuth(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  const authHeader = req.headers["authorization"]?.trim();

  if (!authHeader || !/^Bearer(\s|$)/i.test(authHeader)) {
    res.status(401).json({ error: "Unauthorized - no token provided" });
    return;
  }

  const match = BEARER_JWT_PATTERN.exec(authHeader);

  if (!match) {
    res
      .set("WWW-Authenticate", 'Bearer error="invalid_request"')
      .status(401)
      .json({ error: "Unauthorized - malformed authorization header" });
    return;
  }

  const token = match[1];

  try {
    const payload = jwt.verify(token, config.jwtSecret, { algorithms: ["HS256"] }) as {
      sub: string;
      role: string;
      jti?: string;
    };

    if (!payload.jti) {
      res.status(401).json({ error: "Unauthorized - token has no jti claim" });
      return;
    }

    const revoked = await isTokenRevoked(payload.jti);

    if (revoked) {
      res
        .status(401)
        .json({ error: "Unauthorized - token has been revoked" });
      return;
    }

    req.user = {
      sub: payload.sub,
      role: payload.role,
      jti: payload.jti,
    };

    next();
  } catch {
    res.status(401).json({ error: "Unauthorized - invalid token" });
  }
}

export function requireRole(allowedRoles: string[]) {
  return function (req: Request, res: Response, next: NextFunction): void {
    if (!req.user || !req.user.role) {
      res.status(401).json({ error: "Unauthorized - no user role" });
      return;
    }

    if (!allowedRoles.includes(req.user.role)) {
      res.status(403).json({ error: "Forbidden - insufficient role" });
      return;
    }

    next();
  };
}

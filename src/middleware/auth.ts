import jwt, { JwtPayload } from "jsonwebtoken";
import type { RequestHandler } from "express";
import { prisma } from "../db";

export interface UserIdentity {
  sub: string;
  role: string;
  jti: string;
  sid?: string;
}

const SESSION_CACHE_TTL_MS = 2_000;
const sessionRevocationCache = new Map<string, { revoked: boolean; expiresAt: number }>();

export function clearSessionRevocationCache(sessionIds: string[]): void {
  for (const sessionId of sessionIds) {
    sessionRevocationCache.delete(sessionId);
  }
}

export async function isTokenRevoked(jti: string): Promise<boolean> {
  return Boolean(await prisma.revokedToken.findUnique({ where: { jti } }));
}

export async function isSessionRevoked(sid: string): Promise<boolean> {
  const cached = sessionRevocationCache.get(sid);
  if (cached && cached.expiresAt > Date.now()) {
    return cached.revoked;
  }

  const session = await prisma.session.findUnique({
    where: { id: sid },
    select: { revokedAt: true },
  });
  const revoked = !session || session.revokedAt !== null;
  sessionRevocationCache.set(sid, {
    revoked,
    expiresAt: Date.now() + SESSION_CACHE_TTL_MS,
  });
  return revoked;
}

function getJwtSecret(): string {
  const secret = process.env.JWT_SECRET;
  if (!secret) {
    throw new Error("JWT_SECRET is not configured");
  }
  return secret;
}

export const requireAuth: RequestHandler = async (req, res, next) => {
  const authHeader = req.headers.authorization;
  if (!authHeader?.startsWith("Bearer ")) {
    res.status(401).json({ error: "Unauthorized - no token provided" });
    return;
  }

  let secret: string;
  try {
    secret = getJwtSecret();
  } catch (error) {
    next(error);
    return;
  }

  let payload: JwtPayload;
  try {
    const verified = jwt.verify(authHeader.slice(7), secret);
    if (typeof verified === "string") {
      res.status(401).json({ error: "Unauthorized - invalid token" });
      return;
    }
    payload = verified;
  } catch {
    res.status(401).json({ error: "Unauthorized - invalid token" });
    return;
  }

  if (
    typeof payload.sub !== "string" ||
    typeof payload.role !== "string" ||
    typeof payload.jti !== "string"
  ) {
    res.status(401).json({ error: "Unauthorized - token is missing required claims" });
    return;
  }

  try {
    if (await isTokenRevoked(payload.jti)) {
      res.status(401).json({ error: "Unauthorized - token has been revoked" });
      return;
    }
    if (typeof payload.sid === "string" && await isSessionRevoked(payload.sid)) {
      res.status(401).json({ error: "Unauthorized - session has been revoked" });
      return;
    }

    req.user = {
      sub: payload.sub,
      role: payload.role,
      jti: payload.jti,
      ...(typeof payload.sid === "string" ? { sid: payload.sid } : {}),
    };
    next();
  } catch (error) {
    next(error);
  }
};

export function requireRole(allowedRoles: string[]): RequestHandler {
  return (req, res, next) => {
    if (!req.user?.role) {
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

export function getConfiguredJwtSecret(): string {
  return getJwtSecret();
}

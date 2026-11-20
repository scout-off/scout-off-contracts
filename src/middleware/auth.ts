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

// Sentinel role assigned to wallets that have not registered as any known role.
// Tokens carrying this role are only accepted by routes that explicitly opt in
// via `requireAuth({ allowUnknown: true })`.
export const UNKNOWN_ROLE = "unknown";

export interface RequireAuthOptions {
  /**
   * When true, tokens whose role is `UNKNOWN_ROLE` are accepted. Defaults to
   * false so protected routes reject unregistered wallets unless they opt in.
   */
  allowUnknown?: boolean;
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

export async function requireAuth(
  req: Request,
  res: Response,
  next: NextFunction,
  options: RequireAuthOptions = {}
): Promise<void> {
  const { allowUnknown = false } = options;
  const authHeader = req.headers["authorization"];

  if (!authHeader || !authHeader.startsWith("Bearer ")) {
    res.status(401).json({ error: "Unauthorized - no token provided" });
    return;
  }

  const token = authHeader.split(" ")[1];

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

    if (payload.role === UNKNOWN_ROLE && !allowUnknown) {
      res.status(403).json({ error: "Forbidden - wallet not registered" });
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

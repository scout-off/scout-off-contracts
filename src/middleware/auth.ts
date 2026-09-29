import jwt from "jsonwebtoken";
import { Request, Response, NextFunction } from "express";
import { prisma } from "../db";

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

export async function requireAuth(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  const authHeader = req.headers["authorization"];

  if (!authHeader || !authHeader.startsWith("Bearer ")) {
    res.status(401).json({ error: "Unauthorized - no token provided" });
    return;
  }

  const token = authHeader.split(" ")[1];

  try {
    const payload = jwt.verify(token, process.env.JWT_SECRET!) as {
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

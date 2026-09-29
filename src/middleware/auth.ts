import jwt from "jsonwebtoken";
import { PrismaClient } from "@prisma/client";

const prisma = new PrismaClient();

export async function isTokenRevoked(jti: string): Promise<boolean> {
  if (!jti) {
    return false;
  }

  const existingRevocation = await prisma.revoked_tokens.findFirst({
    where: { jti },
  });

  return !!existingRevocation;
}

export function requireAuth(
  req: Request,
  res: any,
  next: () => void
) {
  const authHeader = req.headers["authorization"];

  if (!authHeader || !authHeader.startsWith("Bearer ")) {
    return res.status(401).json({ error: "Unauthorized - no token provided" });
  }

  const token = authHeader.split(" ")[1];

  try {
    const payload = jwt.verify(token, process.env.JWT_SECRET!) as {
      sub: string;
      role: string;
      jti?: string;
    };

    if (!payload.jti) {
      return res.status(401).json({ error: "Unauthorized - token has no jti claim" });
    }

    const isRevoked = await isTokenRevoked(payload.jti);

    if (isRevoked) {
      return res.status(401).json({ error: "Unauthorized - token has been revoked" });
    }

    req.user = {
      sub: payload.sub,
      role: payload.role,
      jti: payload.jti,
    };

    next();
  } catch (error) {
    return res.status(401).json({ error: "Unauthorized - invalid token" });
  }
}

export function requireRole(
  allowedRoles: string[]
) {
  return function (
    req: Request,
    res: any,
    next: () => void
  ) {
    if (!req.user || !req.user.role) {
      return res.status(401).json({ error: "Unauthorized - no user role" });
    }

    if (!allowedRoles.includes(req.user.role)) {
      return res.status(403).json({ error: "Forbidden - insufficient role" });
    }

    next();
  };
}
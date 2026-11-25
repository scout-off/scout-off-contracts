import { jwtVerify } from "jose";
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

  const existingRevocation = await prisma.revokedToken.findUnique({
    where: { jti },
  });

  return !!existingRevocation;
}

/**
 * Verify a JWT using the active signing configuration.
 *
 * - Asymmetric (EdDSA): look up the public key by `kid`; reject tokens with
 *   an unknown or missing `kid`.
 * - Migration mode (HS256 fallback): accepted only when
 *   `config.allowHs256Migration` is true.
 * - `alg: none` and cross-algorithm confusion are prevented by pinning
 *   the allowed algorithms list.
 */
async function verifyToken(token: string): Promise<{
  sub: string;
  role: string;
  jti?: string;
}> {
  const { signingKeys, allowHs256Migration, jwtSecret } = config;

  if (signingKeys) {
    // Decode header to pick the right public key by kid (before full verify).
    const parts = token.split(".");
    if (parts.length !== 3) throw new Error("malformed token");
    const header = JSON.parse(Buffer.from(parts[0], "base64url").toString());
    const kid = typeof header.kid === "string" ? header.kid : undefined;
    if (!kid) throw new Error("missing kid");
    const pubKey = signingKeys.publicKeys.get(kid);
    if (!pubKey) throw new Error(`unknown kid: ${kid}`);
    const { payload } = await jwtVerify(token, pubKey, { algorithms: ["EdDSA"] });
    return {
      sub: payload.sub as string,
      role: payload.role as string,
      jti: payload.jti as string | undefined,
    };
  }

  if (allowHs256Migration) {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const jwt = require("jsonwebtoken") as typeof import("jsonwebtoken");
    const payload = jwt.verify(token, jwtSecret, {
      algorithms: ["HS256"],
    }) as { sub: string; role: string; jti?: string };
    return payload;
  }

  throw new Error("No verification key available");
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
    const payload = await verifyToken(token);

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


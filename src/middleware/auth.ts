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
  return Boolean(await prisma.revokedToken.findUnique({ where: { jti } }));
}

export async function isSessionRevoked(sid: string): Promise<boolean> {
  const cached = sessionRevocationCache.get(sid);
  if (cached && cached.expiresAt > Date.now()) {
    return cached.revoked;
  }

  const existingRevocation = await prisma.revokedToken.findUnique({
    where: { jti },
  });
  const revoked = !session || session.revokedAt !== null;
  sessionRevocationCache.set(sid, {
    revoked,
    expiresAt: Date.now() + SESSION_CACHE_TTL_MS,
  });
  return revoked;
}

// RFC 6750: the auth scheme is case-insensitive; tolerate extra whitespace
// before the token. The token must be a compact JWT (three segments).
const BEARER_JWT_PATTERN =
  /^Bearer\s+([A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]*)$/i;

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
export async function verifyToken(token: string): Promise<{
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
    const { payload } = await jwtVerify(token, pubKey, {
      algorithms: ["EdDSA"],
      issuer: config.jwtIssuer || undefined,
      audience: config.jwtAudience || undefined,
    });
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
      issuer: config.jwtIssuer || undefined,
      audience: config.jwtAudience || undefined,
    }) as { sub: string; role: string; jti?: string };
    return payload;
  }

  throw new Error("No verification key available");
}

export async function requireAuth(
  req: Request,
  res: Response,
  next: NextFunction,
  options: RequireAuthOptions = {}
): Promise<void> {
  const { allowUnknown = false } = options;
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
  return secret;
}

  const token = match[1];

  let secret: string;
  try {
    const payload = await verifyToken(token);

    if (!payload.jti) {
      res.status(401).json({ error: "Unauthorized - token has no jti claim" });
      return;
    }
    payload = verified;
  } catch {
    res.status(401).json({ error: "Unauthorized - invalid token" });
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

    if (payload.role === UNKNOWN_ROLE && !allowUnknown) {
      res.status(403).json({ error: "Forbidden - wallet not registered" });
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
      ...(typeof payload.sid === "string" ? { sid: payload.sid } : {}),
    };
    next();
  } catch {
    res.status(401).json({ error: "Unauthorized - invalid token" });
  }
};

export function requireRole(roles: readonly string[]): RequestHandler {
  return (req, res, next) => {
    if (!req.user) {
      res.status(401).json({ error: "Unauthorized" });
      return;
    }
    if (!allowedRoles.includes(req.user.role)) {
      res.status(403).json({ error: "Forbidden - insufficient role" });
      return;
    }
    next();
  };
}

import { createHash, randomBytes, randomUUID } from "node:crypto";
import jwt from "jsonwebtoken";
import { prisma } from "../db";
import { clearSessionRevocationCache } from "../middleware/auth";

const REFRESH_TOKEN_BYTES = 48;

export class SessionError extends Error {
  constructor(
    public readonly code: "invalid_refresh_token" | "refresh_token_reused",
  ) {
    super(code);
  }
}

function hashRefreshToken(token: string): string {
  return createHash("sha256").update(token).digest("hex");
}

function newRefreshToken(): string {
  return randomBytes(REFRESH_TOKEN_BYTES).toString("base64url");
}

export function issueAccessToken(
  identity: { sub: string; role: string; sid: string },
  secret: string,
): string {
  return jwt.sign(identity, secret, {
    expiresIn: "15m",
    jwtid: randomUUID(),
  });
}

export async function createSession(
  account: string,
  role: string,
  secret: string,
): Promise<{ accessToken: string; refreshToken: string }> {
  const sessionId = randomUUID();
  const refreshToken = newRefreshToken();
  const now = new Date();

  await prisma.$transaction(async (tx) => {
    await tx.session.create({
      data: {
        id: sessionId,
        account,
        refreshTokens: {
          create: {
            id: hashRefreshToken(refreshToken),
            role,
            createdAt: now,
          },
        },
      },
    });
  });

  return {
    accessToken: issueAccessToken({ sub: account, role, sid: sessionId }, secret),
    refreshToken,
  };
}

async function revokeFamily(sessionId: string, reason: string): Promise<void> {
  const now = new Date();
  await prisma.$transaction(async (tx) => {
    await tx.session.updateMany({
      where: { id: sessionId, revokedAt: null },
      data: { revokedAt: now, revokedReason: reason },
    });
    await tx.refreshToken.updateMany({
      where: { sessionId, revokedAt: null },
      data: { revokedAt: now },
    });
  });
  clearSessionRevocationCache([sessionId]);
}

export async function revokeSession(
  sessionId: string,
  account: string,
  reason: string,
): Promise<boolean> {
  const session = await prisma.session.findUnique({
    where: { id: sessionId },
    select: { id: true, account: true },
  });
  if (!session || session.account !== account) {
    return false;
  }
  await revokeFamily(sessionId, reason);
  return true;
}

export async function revokeAccountSessions(
  account: string,
  reason: string,
): Promise<number> {
  const now = new Date();
  const sessions = await prisma.session.findMany({
    where: { account, revokedAt: null },
    select: { id: true },
  });
  if (sessions.length === 0) {
    return 0;
  }

  const sessionIds = sessions.map(({ id }) => id);
  const result = await prisma.$transaction(async (tx) => {
    const revoked = await tx.session.updateMany({
      where: { account, revokedAt: null },
      data: { revokedAt: now, revokedReason: reason },
    });
    await tx.refreshToken.updateMany({
      where: { session: { is: { account } }, revokedAt: null },
      data: { revokedAt: now },
    });
    return revoked.count;
  });
  clearSessionRevocationCache(sessionIds);
  return result;
}

export async function revokeSessionById(
  sessionId: string,
  reason: string,
): Promise<boolean> {
  const session = await prisma.session.findUnique({
    where: { id: sessionId },
    select: { account: true },
  });
  return session ? revokeSession(sessionId, session.account, reason) : false;
}

export async function refreshSession(
  presentedToken: string,
  secret: string,
): Promise<{ accessToken: string; refreshToken: string }> {
  if (!presentedToken || typeof presentedToken !== "string") {
    throw new SessionError("invalid_refresh_token");
  }

  const tokenHash = hashRefreshToken(presentedToken);
  const current = await prisma.refreshToken.findUnique({
    where: { id: tokenHash },
    include: { session: true },
  });
  if (!current) {
    throw new SessionError("invalid_refresh_token");
  }

  if (current.rotatedAt) {
    await revokeFamily(current.sessionId, "refresh_token_reused");
    throw new SessionError("refresh_token_reused");
  }
  if (current.revokedAt || current.session.revokedAt) {
    throw new SessionError("invalid_refresh_token");
  }

  const nextToken = newRefreshToken();
  const nextHash = hashRefreshToken(nextToken);
  const now = new Date();
  const rotated = await prisma.$transaction(async (tx) => {
    const stillActive = await tx.session.findUnique({
      where: { id: current.sessionId },
      select: { revokedAt: true },
    });
    if (!stillActive || stillActive.revokedAt) {
      return false;
    }

    const claimed = await tx.refreshToken.updateMany({
      where: {
        id: tokenHash,
        revokedAt: null,
        rotatedAt: null,
        session: { is: { revokedAt: null } },
      },
      data: { revokedAt: now, rotatedAt: now, replacedBy: nextHash },
    });
    if (claimed.count !== 1) {
      return false;
    }
    await tx.refreshToken.create({
      data: {
        id: nextHash,
        sessionId: current.sessionId,
        role: current.role,
        createdAt: now,
      },
    });
    return true;
  });

  if (!rotated) {
    const latest = await prisma.refreshToken.findUnique({
      where: { id: tokenHash },
      select: { rotatedAt: true, sessionId: true },
    });
    if (latest?.rotatedAt) {
      await revokeFamily(latest.sessionId, "refresh_token_reused");
      throw new SessionError("refresh_token_reused");
    }
    throw new SessionError("invalid_refresh_token");
  }

  return {
    accessToken: issueAccessToken({
      sub: current.session.account,
      role: current.role,
      sid: current.sessionId,
    }, secret),
    refreshToken: nextToken,
  };
}

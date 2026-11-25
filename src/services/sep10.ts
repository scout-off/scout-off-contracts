import jwt from "jsonwebtoken";
import { randomUUID } from "node:crypto";

interface Sep10TokenPayload {
  sub: string;
  role: string;
}

export function issueSep10Token(payload: Sep10TokenPayload, secret: string) {
  return jwt.sign(payload, secret, {
    expiresIn: "15m",
    jwtid: randomUUID(),
  });
}
import { v4 as uuidv4 } from "uuid";
import jwt from "jsonwebtoken";
import type { Request } from "express";

interface Sep10TokenPayload {
  sub: string;
  role: string;
}

export function issueSep10Token(payload: Sep10TokenPayload, secret: string) {
  return jwt.sign(payload, secret, {
    expiresIn: "7d",
    jwtid: uuidv4(),
  });
}
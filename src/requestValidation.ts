import { Request, Response, NextFunction } from "express";
import { z, ZodTypeAny } from "zod";
import { StrKey } from "@stellar/stellar-sdk";

/**
 * Request validation. Unknown keys are STRIPPED (zod object default): parsed
 * values replace `req.query` / `req.body` so handlers only see declared fields.
 * Failures respond `400 { error: "invalid_request", details: [{ path, message }] }`.
 */

/** Max length of a base64 SEP-10 challenge transaction (~10 KB). */
export const MAX_TRANSACTION_LENGTH = 10 * 1024;

const stellarAccount = z
  .string({ required_error: "account is required", invalid_type_error: "account must be a string" })
  .refine((v) => StrKey.isValidEd25519PublicKey(v), "invalid Stellar account");

const base64 = /^[A-Za-z0-9+/]+={0,2}$/;
const jwtShape = /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$/;

export const AccountQuery = z.object({ account: stellarAccount });

export const VerifyBody = z.object({
  transaction: z
    .string({ required_error: "transaction is required", invalid_type_error: "transaction must be a string" })
    .min(1, "transaction is required")
    .max(MAX_TRANSACTION_LENGTH, `transaction exceeds ${MAX_TRANSACTION_LENGTH} characters`)
    .regex(base64, "transaction must be base64"),
  account: stellarAccount,
});

export const RefreshBody = z.object({
  refreshToken: z
    .string({ required_error: "refreshToken is required", invalid_type_error: "refreshToken must be a string" })
    .min(1, "refreshToken is required")
    .max(4096, "refreshToken too long"),
});

export const RevokeBody = z.object({
  token: z
    .string({ required_error: "token is required", invalid_type_error: "token must be a string" })
    .max(8192, "token too long")
    .regex(jwtShape, "token must be a JWT"),
});

export function validate(schemas: { query?: ZodTypeAny; body?: ZodTypeAny }) {
  return (req: Request, res: Response, next: NextFunction) => {
    for (const key of ["query", "body"] as const) {
      const schema = schemas[key];
      if (!schema) continue;
      const result = schema.safeParse(req[key] ?? {});
      if (!result.success) {
        return res.status(400).json({
          error: "invalid_request",
          details: result.error.issues.map((i) => ({
            path: i.path.join("."),
            message: i.message,
          })),
        });
      }
      (req as unknown as Record<string, unknown>)[key] = result.data;
    }
    next();
  };
}

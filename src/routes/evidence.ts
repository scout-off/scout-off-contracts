import { Router, type Request, type RequestHandler } from "express";
import rateLimit from "express-rate-limit";
import { StrKey } from "@stellar/stellar-base";
import { decryptEvidenceDataKey, wrapEvidenceDataKey } from "../services/evidenceKeyCrypto";
import type { EvidenceGrantChecker } from "../services/evidenceGrantChecker";

interface QueryResult<Row> {
  rows: Row[];
  rowCount: number | null;
}

export interface EvidenceDatabase {
  query<Row extends Record<string, unknown> = Record<string, unknown>>(
    text: string,
    values?: unknown[]
  ): Promise<QueryResult<Row>>;
}

export interface EvidenceRouteDependencies {
  database: EvidenceDatabase;
  grantChecker: EvidenceGrantChecker;
  masterKey: Buffer;
  authenticate: RequestHandler;
}

interface EncryptedKeyRow extends Record<string, unknown> {
  encrypted_data_key: Buffer;
  nonce: Buffer;
  auth_tag: Buffer;
}

function isPositiveInteger(value: string, maximum: bigint): boolean {
  return /^[1-9]\d*$/.test(value) && BigInt(value) <= maximum;
}

export function createEvidenceRouter(
  dependencies: EvidenceRouteDependencies
): Router {
  const router = Router();
  const keyReleaseLimit = rateLimit({
    windowMs: 15 * 60 * 1000,
    limit: 30,
    standardHeaders: "draft-8",
    legacyHeaders: false,
  });

  router.post(
    "/:playerId/:milestoneIndex/key",
    keyReleaseLimit,
    dependencies.authenticate,
    async (req: Request, res, next) => {
      try {
        const playerId = req.params.playerId;
        const milestoneIndex = req.params.milestoneIndex;
        if (
          typeof playerId !== "string" ||
          typeof milestoneIndex !== "string" ||
          !isPositiveInteger(playerId, 9223372036854775807n) ||
          !isPositiveInteger(milestoneIndex, 2147483647n) ||
          !req.user
        ) {
          res.status(400).json({ error: "Invalid evidence request" });
          return;
        }

        const { sub: wallet, role } = req.user;
        if (!StrKey.isValidEd25519PublicKey(wallet)) {
          res.status(403).json({ error: "Forbidden" });
          return;
        }

        if (role === "scout") {
          if (
            !(await dependencies.grantChecker.hasEvidenceAccess(playerId, wallet))
          ) {
            res.status(403).json({ error: "Evidence access denied" });
            return;
          }
        } else if (role === "player") {
          const owner = await dependencies.database.query<{ wallet: string }>(
            "SELECT wallet FROM players WHERE player_id = $1",
            [playerId]
          );
          if (owner.rows[0]?.wallet !== wallet) {
            res.status(403).json({ error: "Evidence access denied" });
            return;
          }
        } else {
          res.status(403).json({ error: "Forbidden" });
          return;
        }

        const keyRecord =
          await dependencies.database.query<EncryptedKeyRow>(
            `SELECT encrypted_data_key, nonce, auth_tag
             FROM evidence_data_keys
             WHERE player_id = $1 AND milestone_index = $2`,
            [playerId, milestoneIndex]
          );
        const encrypted = keyRecord.rows[0];
        if (!encrypted) {
          res.status(404).json({ error: "Evidence key not found" });
          return;
        }

        const dataKey = decryptEvidenceDataKey(
          {
            ciphertext: encrypted.encrypted_data_key,
            nonce: encrypted.nonce,
            authTag: encrypted.auth_tag,
          },
          dependencies.masterKey,
          playerId,
          milestoneIndex
        );
        let wrappedKey: string;
        try {
          wrappedKey = await wrapEvidenceDataKey(dataKey, wallet);
        } finally {
          dataKey.fill(0);
        }

        await dependencies.database.query(
          `INSERT INTO evidence_key_releases
             (player_id, milestone_index, requester_wallet, requester_role,
              grant_expires_at)
           VALUES ($1, $2, $3, $4, NULL)`,
          [playerId, milestoneIndex, wallet, role]
        );

        res.json({
          algorithm: "crypto_box_seal_curve25519xsalsa20poly1305",
          recipient: wallet,
          wrappedKey,
        });
      } catch (error) {
        next(error);
      }
    }
  );

  return router;
}

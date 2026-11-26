import type { EncryptedDataKey } from "./evidenceKeyCrypto";
import { encryptEvidenceDataKey } from "./evidenceKeyCrypto";

export interface EvidenceKeyStore {
  query(
    text: string,
    values?: unknown[]
  ): Promise<{ rowCount: number | null }>;
}

export async function storeEvidenceDataKey(
  database: EvidenceKeyStore,
  masterKey: Buffer,
  playerId: string,
  milestoneIndex: string,
  dataKey: Buffer
): Promise<void> {
  const encrypted: EncryptedDataKey = encryptEvidenceDataKey(
    dataKey,
    masterKey,
    playerId,
    milestoneIndex
  );
  const result = await database.query(
    `INSERT INTO evidence_data_keys
       (player_id, milestone_index, encrypted_data_key, nonce, auth_tag)
     VALUES ($1, $2, $3, $4, $5)
     ON CONFLICT (player_id, milestone_index) DO NOTHING`,
    [
      playerId,
      milestoneIndex,
      encrypted.ciphertext,
      encrypted.nonce,
      encrypted.authTag,
    ]
  );
  if (result.rowCount !== 1) {
    throw new Error("An evidence data key already exists for this milestone");
  }
}

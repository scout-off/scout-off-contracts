import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto";
import { StrKey } from "@stellar/stellar-base";
import sodium from "libsodium-wrappers";

const DATA_KEY_BYTES = 32;
const NONCE_BYTES = 12;

export interface EncryptedDataKey {
  ciphertext: Buffer;
  nonce: Buffer;
  authTag: Buffer;
}

function evidenceAad(playerId: string, milestoneIndex: string): Buffer {
  return Buffer.from(`evidence-key:${playerId}:${milestoneIndex}`, "utf8");
}

export function parseMasterKey(encodedKey: string | undefined): Buffer {
  if (!encodedKey) {
    throw new Error("EVIDENCE_MASTER_KEY is required");
  }
  const key = Buffer.from(encodedKey, "base64");
  if (key.length !== DATA_KEY_BYTES || key.toString("base64") !== encodedKey) {
    throw new Error("EVIDENCE_MASTER_KEY must be a base64-encoded 32-byte key");
  }
  return key;
}

export function encryptEvidenceDataKey(
  dataKey: Buffer,
  masterKey: Buffer,
  playerId: string,
  milestoneIndex: string
): EncryptedDataKey {
  if (dataKey.length !== DATA_KEY_BYTES || masterKey.length !== DATA_KEY_BYTES) {
    throw new Error("Evidence keys must be 32 bytes");
  }

  const nonce = randomBytes(NONCE_BYTES);
  const cipher = createCipheriv("aes-256-gcm", masterKey, nonce);
  cipher.setAAD(evidenceAad(playerId, milestoneIndex));
  const ciphertext = Buffer.concat([cipher.update(dataKey), cipher.final()]);

  return { ciphertext, nonce, authTag: cipher.getAuthTag() };
}

export function decryptEvidenceDataKey(
  encrypted: EncryptedDataKey,
  masterKey: Buffer,
  playerId: string,
  milestoneIndex: string
): Buffer {
  if (
    masterKey.length !== DATA_KEY_BYTES ||
    encrypted.nonce.length !== NONCE_BYTES ||
    encrypted.authTag.length !== 16
  ) {
    throw new Error("Invalid encrypted evidence key");
  }

  const decipher = createDecipheriv("aes-256-gcm", masterKey, encrypted.nonce);
  decipher.setAAD(evidenceAad(playerId, milestoneIndex));
  decipher.setAuthTag(encrypted.authTag);
  const dataKey = Buffer.concat([
    decipher.update(encrypted.ciphertext),
    decipher.final(),
  ]);
  if (dataKey.length !== DATA_KEY_BYTES) {
    throw new Error("Invalid decrypted evidence key");
  }
  return dataKey;
}

export async function wrapEvidenceDataKey(
  dataKey: Buffer,
  stellarAddress: string
): Promise<string> {
  if (dataKey.length !== DATA_KEY_BYTES) {
    throw new Error("Evidence keys must be 32 bytes");
  }
  if (!StrKey.isValidEd25519PublicKey(stellarAddress)) {
    throw new Error("Invalid Stellar public key");
  }

  await sodium.ready;
  const ed25519PublicKey = StrKey.decodeEd25519PublicKey(stellarAddress);
  const curve25519PublicKey =
    sodium.crypto_sign_ed25519_pk_to_curve25519(ed25519PublicKey);
  const sealed = sodium.crypto_box_seal(dataKey, curve25519PublicKey);
  return Buffer.from(sealed).toString("base64");
}

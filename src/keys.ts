import {
  createPrivateKey,
  createPublicKey,
  generateKeyPairSync,
  KeyObject,
} from "crypto";

/** A signing key set: one active private key plus any previous public keys. */
export interface SigningKeys {
  /** Active `kid` used for signing. */
  activeKid: string;
  /** Active private key (signing). */
  activePrivateKey: KeyObject;
  /** All public keys by `kid` (verification / JWKS). */
  publicKeys: Map<string, KeyObject>;
  algorithm: "EdDSA";
}

/**
 * Parse `JWT_SIGNING_KEYS`: entries separated by newlines or semicolons, each
 * `kid,<PEM>`. At least one entry must be a private key (the active signer);
 * additional entries are previous public keys kept for rotation overlap.
 * `JWT_SIGNING_ACTIVE_KID` selects the active kid when multiple private
 * keys are present (otherwise the first private key wins).
 */
export function loadSigningKeys(env: NodeJS.ProcessEnv): SigningKeys | null {
  const raw = env.JWT_SIGNING_KEYS?.trim();
  if (!raw) return null;

  const entries = raw
    .split(/[;\n]+/)
    .map((e) => e.trim())
    .filter(Boolean);

  const publicKeys = new Map<string, KeyObject>();
  let preferredKid = env.JWT_SIGNING_ACTIVE_KID?.trim() || "";
  let activeKid = "";
  let activePrivateKey: KeyObject | null = null;

  for (const entry of entries) {
    const comma = entry.indexOf(",");
    if (comma < 0) continue;
    const kid = entry.slice(0, comma).trim();
    const pem = entry
      .slice(comma + 1)
      .replace(/\\n/g, "\n")
      .trim();
    if (!kid || !pem) continue;

    try {
      const priv = createPrivateKey(pem);
      const pub = createPublicKey(priv);
      publicKeys.set(kid, pub);
      // Use the first private key found, or the one matching the preferred kid.
      if (!activePrivateKey || kid === preferredKid) {
        activePrivateKey = priv;
        activeKid = kid;
      }
    } catch {
      try {
        const pub = createPublicKey(pem);
        publicKeys.set(kid, pub);
      } catch {
        throw new Error(
          `key '${kid}' is neither a valid private nor public PEM`
        );
      }
    }
  }

  if (publicKeys.size === 0) {
    throw new Error("no usable keys");
  }
  if (!activePrivateKey) {
    throw new Error("must include one private (active) key for signing");
  }
  if (preferredKid && !publicKeys.has(preferredKid)) {
    throw new Error(
      `JWT_SIGNING_ACTIVE_KID '${preferredKid}' has no matching key`
    );
  }

  return {
    activeKid,
    activePrivateKey,
    publicKeys,
    algorithm: "EdDSA",
  };
}

/** Generate a new Ed25519 key pair for rotation; returns PEM strings. */
export function generateSigningKeyPair(): {
  privateKeyPem: string;
  publicKeyPem: string;
} {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  return {
    privateKeyPem: privateKey
      .export({ type: "pkcs8", format: "pem" })
      .toString(),
    publicKeyPem: publicKey.export({ type: "spki", format: "pem" }).toString(),
  };
}

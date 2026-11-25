import { StrKey } from "@stellar/stellar-sdk";
import { loadSigningKeys, SigningKeys } from "./keys";

export const SUPPORTED_NETWORKS = ["testnet", "mainnet"] as const;
export type StellarNetwork = (typeof SUPPORTED_NETWORKS)[number];

export interface Config {
  /** Legacy HS256 secret – retained only when JWT_ALLOW_HS256_MIGRATION=true. */
  jwtSecret: string;
  /** Asymmetric signing keys (required unless migration flag is set). */
  signingKeys: SigningKeys | null;
  /**
   * When true, the server falls back to HS256 so existing deployments can
   * migrate. Set JWT_ALLOW_HS256_MIGRATION=true only during the transition.
   */
  allowHs256Migration: boolean;
  sep10SigningSecret: string;
  homeDomain: string;
  webAuthDomain: string;
  stellarNetwork: StellarNetwork;
  adminAddress?: string;
}

const MIN_JWT_SECRET_BYTES = 32;

/**
 * Validate and parse the backend's environment variables.
 *
 * Throws a single error listing every missing or invalid variable so the
 * server fails fast at startup instead of per request.
 */
export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const errors: string[] = [];

  const required = (name: string): string => {
    const value = env[name]?.trim();
    if (!value) {
      errors.push(`${name} is required`);
      return "";
    }
    return value;
  };

  const allowHs256Migration =
    env.JWT_ALLOW_HS256_MIGRATION?.trim().toLowerCase() === "true";

  // JWT_SECRET is required during migration; optional (but accepted) otherwise.
  const jwtSecretRaw = env.JWT_SECRET?.trim() || "";
  if (allowHs256Migration && !jwtSecretRaw) {
    errors.push("JWT_SECRET is required when JWT_ALLOW_HS256_MIGRATION=true");
  }
  if (
    jwtSecretRaw &&
    Buffer.byteLength(jwtSecretRaw, "utf8") < MIN_JWT_SECRET_BYTES
  ) {
    errors.push(`JWT_SECRET must be at least ${MIN_JWT_SECRET_BYTES} bytes`);
  }

  // Asymmetric signing keys – required unless migration mode is active.
  let signingKeys: SigningKeys | null = null;
  try {
    signingKeys = loadSigningKeys(env);
  } catch (err) {
    errors.push(
      `JWT_SIGNING_KEYS: ${err instanceof Error ? err.message : String(err)}`
    );
  }

  if (!signingKeys && !allowHs256Migration) {
    errors.push(
      "JWT_SIGNING_KEYS is required. " +
        "Set JWT_ALLOW_HS256_MIGRATION=true to keep HS256 during migration."
    );
  }

  const sep10SigningSecret = required("SEP10_SIGNING_SECRET");
  if (sep10SigningSecret && !StrKey.isValidEd25519SecretSeed(sep10SigningSecret)) {
    errors.push("SEP10_SIGNING_SECRET must be a valid Stellar secret key (S...)");
  }

  const homeDomain = required("HOME_DOMAIN");
  const webAuthDomain = required("WEB_AUTH_DOMAIN");

  const network = env.STELLAR_NETWORK?.trim() || "testnet";
  if (!(SUPPORTED_NETWORKS as readonly string[]).includes(network)) {
    errors.push(`STELLAR_NETWORK must be one of: ${SUPPORTED_NETWORKS.join(", ")}`);
  }

  if (errors.length > 0) {
    throw new Error(`Invalid environment configuration:\n  - ${errors.join("\n  - ")}`);
  }

  return {
    jwtSecret: jwtSecretRaw,
    signingKeys,
    allowHs256Migration,
    sep10SigningSecret,
    homeDomain,
    webAuthDomain,
    stellarNetwork: network as StellarNetwork,
    adminAddress: env.ADMIN_ADDRESS?.trim() || undefined,
  };
}

export const config = loadConfig();

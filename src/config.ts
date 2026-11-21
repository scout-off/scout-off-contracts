import { StrKey } from "@stellar/stellar-sdk";

export const SUPPORTED_NETWORKS = ["testnet", "mainnet"] as const;
export type StellarNetwork = (typeof SUPPORTED_NETWORKS)[number];

export interface Config {
  jwtSecret: string;
  sep10SigningSecret: string;
  homeDomain: string;
  webAuthDomain: string;
  stellarNetwork: StellarNetwork;
  adminAddress?: string;
  accessTokenTtlSeconds: number;
  refreshTokenTtlSeconds: number;
}

const MIN_JWT_SECRET_BYTES = 32;

export const DEFAULT_ACCESS_TOKEN_TTL_SECONDS = 15 * 60;
export const DEFAULT_REFRESH_TOKEN_TTL_SECONDS = 7 * 24 * 60 * 60;
export const MAX_ACCESS_TOKEN_TTL_SECONDS = 15 * 60;
=======

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

  const jwtSecret = required("JWT_SECRET");
  if (jwtSecret && Buffer.byteLength(jwtSecret, "utf8") < MIN_JWT_SECRET_BYTES) {
    errors.push(`JWT_SECRET must be at least ${MIN_JWT_SECRET_BYTES} bytes`);
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

  const positiveInteger = (name: string, defaultValue: number): number => {
    const raw = env[name]?.trim();
    if (!raw) {
      return defaultValue;
    }
    if (!/^\d+$/.test(raw)) {
      errors.push(`${name} must be a positive integer (seconds), got "${raw}"`);
      return defaultValue;
    }
    const value = Number(raw);
    if (!Number.isSafeInteger(value) || value <= 0) {
      errors.push(`${name} must be a positive integer (seconds), got "${raw}"`);
      return defaultValue;
    }
    return value;
  };

  const accessTokenTtlSeconds = positiveInteger(
    "ACCESS_TOKEN_TTL_SECONDS",
    DEFAULT_ACCESS_TOKEN_TTL_SECONDS,
  );
  if (accessTokenTtlSeconds > MAX_ACCESS_TOKEN_TTL_SECONDS) {
    errors.push(
      `ACCESS_TOKEN_TTL_SECONDS must not exceed ${MAX_ACCESS_TOKEN_TTL_SECONDS} seconds ` +
        `(15 minutes); values above the ceiling are rejected, not clamped`,
    );
  }

  const refreshTokenTtlSeconds = positiveInteger(
    "REFRESH_TOKEN_TTL_SECONDS",
    DEFAULT_REFRESH_TOKEN_TTL_SECONDS,
  );

  if (errors.length > 0) {
    throw new Error(`Invalid environment configuration:\n  - ${errors.join("\n  - ")}`);
  }

  return {
    jwtSecret,
    sep10SigningSecret,
    homeDomain,
    webAuthDomain,
    stellarNetwork: network as StellarNetwork,
    adminAddress: env.ADMIN_ADDRESS?.trim() || undefined,
    accessTokenTtlSeconds,
    refreshTokenTtlSeconds,
  };
}

export const config = loadConfig();

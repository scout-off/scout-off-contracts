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
  /**
   * Refresh-token lifetime (in seconds) for wallets whose role is `unknown`,
   * i.e. wallets that have not registered as admin/validator/scout/player.
   * Kept short so unregistered wallets can only hold a session long enough to
   * complete onboarding.
   */
  unknownRefreshTokenTtlSeconds: number;
}

const MIN_JWT_SECRET_BYTES = 32;

const DEFAULT_UNKNOWN_REFRESH_TOKEN_TTL_SECONDS = 60 * 60; // 1 hour
const MAX_UNKNOWN_REFRESH_TOKEN_TTL_SECONDS = 24 * 60 * 60; // 24 hours

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

  const unknownTtlRaw = env.UNKNOWN_REFRESH_TOKEN_TTL_SECONDS?.trim();
  let unknownRefreshTokenTtlSeconds = DEFAULT_UNKNOWN_REFRESH_TOKEN_TTL_SECONDS;
  if (unknownTtlRaw) {
    if (!/^\d+$/.test(unknownTtlRaw)) {
      errors.push("UNKNOWN_REFRESH_TOKEN_TTL_SECONDS must be a positive integer");
    } else {
      const parsed = Number(unknownTtlRaw);
      if (!Number.isSafeInteger(parsed) || parsed <= 0) {
        errors.push("UNKNOWN_REFRESH_TOKEN_TTL_SECONDS must be a positive integer");
      } else if (parsed > MAX_UNKNOWN_REFRESH_TOKEN_TTL_SECONDS) {
        errors.push(
          `UNKNOWN_REFRESH_TOKEN_TTL_SECONDS must be at most ${MAX_UNKNOWN_REFRESH_TOKEN_TTL_SECONDS} (24 hours)`,
        );
      } else {
        unknownRefreshTokenTtlSeconds = parsed;
      }
    }
  }

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
    unknownRefreshTokenTtlSeconds,
  };
}

export const config = loadConfig();

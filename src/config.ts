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
  trustProxy: boolean;
  getAuthIpLimit: number;
  getAuthAccountLimit: number;
  postAuthIpLimit: number;
  postAuthRefreshIpLimit: number;
  adminIpLimit: number;
  maxUnconsumedChallengesPerAccount: number;
  cleanupIntervalSeconds: number;
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

  const trustProxy = env.TRUST_PROXY?.trim()?.toLowerCase() === "true";
  
  const getAuthIpLimit = parseInt(env.GET_AUTH_IP_LIMIT || "10", 10);
  const getAuthAccountLimit = parseInt(env.GET_AUTH_ACCOUNT_LIMIT || "5", 10);
  const postAuthIpLimit = parseInt(env.POST_AUTH_IP_LIMIT || "20", 10);
  const postAuthRefreshIpLimit = parseInt(env.POST_AUTH_REFRESH_IP_LIMIT || "10", 10);
  const adminIpLimit = parseInt(env.ADMIN_IP_LIMIT || "10", 10);
  const maxUnconsumedChallengesPerAccount = parseInt(env.MAX_UNCONSUMED_CHALLENGES_PER_ACCOUNT || "5", 10);
  const cleanupIntervalSeconds = parseInt(env.CLEANUP_INTERVAL_SECONDS || "300", 10); // 5 minutes default

  const network = env.STELLAR_NETWORK?.trim() || "testnet";
  if (!(SUPPORTED_NETWORKS as readonly string[]).includes(network)) {
    errors.push(`STELLAR_NETWORK must be one of: ${SUPPORTED_NETWORKS.join(", ")}`);
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
    trustProxy,
    getAuthIpLimit,
    getAuthAccountLimit,
    postAuthIpLimit,
    postAuthRefreshIpLimit,
    adminIpLimit,
    maxUnconsumedChallengesPerAccount,
    cleanupIntervalSeconds,
  };
}

export const config = loadConfig();

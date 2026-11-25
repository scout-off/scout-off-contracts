import { Networks, StrKey } from "@stellar/stellar-sdk";
import { loadSigningKeys, SigningKeys } from "./keys";

export const SUPPORTED_NETWORKS = ["testnet", "mainnet", "local", "custom"] as const;
export type StellarNetwork = (typeof SUPPORTED_NETWORKS)[number];

/** Networks that require an explicit NETWORK_PASSPHRASE. */
const CUSTOM_PASSPHRASE_NETWORKS: readonly StellarNetwork[] = ["local", "custom"];
export const DEFAULT_JWT_AUDIENCE = "scoutchain-api";

export interface Config {
  /** Legacy HS256 secret – retained only when JWT_ALLOW_HS256_MIGRATION=true. */
  jwtSecret: string;
  jwtIssuer: string;
  jwtAudience: string;
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
  networkPassphrase: string;
  adminAddress?: string;
  /**
   * Refresh-token lifetime (in seconds) for wallets whose role is `unknown`,
   * i.e. wallets that have not registered as admin/validator/scout/player.
   * Kept short so unregistered wallets can only hold a session long enough to
   * complete onboarding.
   */
  unknownRefreshTokenTtlSeconds: number;
  trustProxy: boolean;
  getAuthIpLimit: number;
  getAuthAccountLimit: number;
  postAuthIpLimit: number;
  postAuthRefreshIpLimit: number;
  adminIpLimit: number;
  maxUnconsumedChallengesPerAccount: number;
  cleanupIntervalSeconds: number;
}

export interface JwtPayload {
  sub: string;
  role: string;
  iat?: number;
  exp?: number;
  jti?: string;
  iss?: string;
  aud?: string | string[];
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

  const trustProxy = env.TRUST_PROXY?.trim()?.toLowerCase() === "true";
  
  const getAuthIpLimit = parseInt(env.GET_AUTH_IP_LIMIT || "10", 10);
  const getAuthAccountLimit = parseInt(env.GET_AUTH_ACCOUNT_LIMIT || "5", 10);
  const postAuthIpLimit = parseInt(env.POST_AUTH_IP_LIMIT || "20", 10);
  const postAuthRefreshIpLimit = parseInt(env.POST_AUTH_REFRESH_IP_LIMIT || "10", 10);
  const adminIpLimit = parseInt(env.ADMIN_IP_LIMIT || "10", 10);
  const maxUnconsumedChallengesPerAccount = parseInt(env.MAX_UNCONSUMED_CHALLENGES_PER_ACCOUNT || "5", 10);
  const cleanupIntervalSeconds = parseInt(env.CLEANUP_INTERVAL_SECONDS || "300", 10); // 5 minutes default

  const network = env.STELLAR_NETWORK?.trim() || "testnet";
  const networkIsSupported = (SUPPORTED_NETWORKS as readonly string[]).includes(network);
  if (!networkIsSupported) {
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

  let networkPassphrase = "";
  if (networkIsSupported) {
    const stellarNetwork = network as StellarNetwork;
    if (CUSTOM_PASSPHRASE_NETWORKS.includes(stellarNetwork)) {
      if (stellarNetwork === "local" && env.NODE_ENV?.trim() === "production") {
        errors.push("STELLAR_NETWORK=local is not allowed when NODE_ENV=production");
      }
      networkPassphrase = env.NETWORK_PASSPHRASE?.trim() || "";
      if (!networkPassphrase) {
        errors.push(`NETWORK_PASSPHRASE is required when STELLAR_NETWORK=${stellarNetwork}`);
      }
    } else {
      networkPassphrase =
        env.NETWORK_PASSPHRASE?.trim() ||
        (stellarNetwork === "mainnet" ? Networks.PUBLIC : Networks.TESTNET);
    }
  }

  // localhost / host:port are accepted on the development networks only.
  const isTestnet = network === "testnet" || network === "local";

  /**
   * Validate a value as a bare hostname (optionally with a port).
   *
   * Rejects schemes, paths and whitespace. `localhost` and `host:port` are
   * only allowed on testnet (for local development); on mainnet the value
   * must be a plain hostname.
   */
  const validateHostname = (name: string, value: string): void => {
    if (!value) return;

    const example = name === "HOME_DOMAIN" ? "scoutchain.com" : "auth.scoutchain.com";

    if (value.includes("://")) {
      errors.push(`${name} must be a hostname like ${example} (no scheme or path)`);
      return;
    }
    if (value.includes("/")) {
      errors.push(`${name} must be a hostname like ${example} (no scheme or path)`);
      return;
    }
    if (/\s/.test(value)) {
      errors.push(`${name} must be a hostname like ${example} (no whitespace)`);
      return;
    }

    const host = value.includes(":") ? value.slice(0, value.indexOf(":")) : value;
    const port = value.includes(":") ? value.slice(value.indexOf(":") + 1) : undefined;

    if (port !== undefined) {
      if (!isTestnet) {
        errors.push(
          `${name} must be a hostname like ${example} (no port; ports are only allowed on testnet)`,
        );
        return;
      }
      if (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535) {
        errors.push(`${name} must be a hostname like ${example} (invalid port)`);
        return;
      }
    }

    if (host === "localhost") {
      if (!isTestnet) {
        errors.push(
          `${name} must be a hostname like ${example} (localhost is only allowed on testnet)`,
        );
      }
      return;
    }

    if (!/^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?(\.[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?)*$/.test(host)) {
      errors.push(`${name} must be a hostname like ${example} (no scheme or path)`);
    }
  };

  validateHostname("HOME_DOMAIN", homeDomain);
  validateHostname("WEB_AUTH_DOMAIN", webAuthDomain);

  const adminAddress = env.ADMIN_ADDRESS?.trim() || undefined;
  if (adminAddress) {
    if (StrKey.isValidEd25519SecretSeed(adminAddress)) {
      errors.push(
        "ADMIN_ADDRESS looks like a secret key — use the public G-address instead",
      );
    } else if (!StrKey.isValidEd25519PublicKey(adminAddress)) {
      errors.push("ADMIN_ADDRESS must be a valid Stellar public key (G...)");
    }
  }

  if (errors.length > 0) {
    throw new Error(`Invalid environment configuration:\n  - ${errors.join("\n  - ")}`);
  }

  const jwtIssuer = env.JWT_ISSUER?.trim() || (homeDomain ? `https://${homeDomain}` : "");
  const jwtAudience = env.JWT_AUDIENCE?.trim() || DEFAULT_JWT_AUDIENCE;

  return {
    jwtSecret: jwtSecretRaw,
    signingKeys,
    allowHs256Migration,
    jwtIssuer,
    jwtAudience,
    sep10SigningSecret,
    homeDomain,
    webAuthDomain,
    stellarNetwork: network as StellarNetwork,
    networkPassphrase,
    adminAddress,
    trustProxy,
    getAuthIpLimit,
    getAuthAccountLimit,
    postAuthIpLimit,
    postAuthRefreshIpLimit,
    adminIpLimit,
    maxUnconsumedChallengesPerAccount,
    cleanupIntervalSeconds,
    unknownRefreshTokenTtlSeconds,
  };
}

export const config = loadConfig();

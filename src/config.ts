import { Networks, StrKey } from "@stellar/stellar-sdk";

export const SUPPORTED_NETWORKS = ["testnet", "mainnet", "local", "custom"] as const;
export type StellarNetwork = (typeof SUPPORTED_NETWORKS)[number];

/** Networks that require an explicit NETWORK_PASSPHRASE. */
const CUSTOM_PASSPHRASE_NETWORKS: readonly StellarNetwork[] = ["local", "custom"];

export interface Config {
  jwtSecret: string;
  sep10SigningSecret: string;
  homeDomain: string;
  webAuthDomain: string;
  stellarNetwork: StellarNetwork;
  networkPassphrase: string;
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
  const networkIsSupported = (SUPPORTED_NETWORKS as readonly string[]).includes(network);
  if (!networkIsSupported) {
    errors.push(`STELLAR_NETWORK must be one of: ${SUPPORTED_NETWORKS.join(", ")}`);
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

  if (errors.length > 0) {
    throw new Error(`Invalid environment configuration:\n  - ${errors.join("\n  - ")}`);
  }

  return {
    jwtSecret,
    sep10SigningSecret,
    homeDomain,
    webAuthDomain,
    stellarNetwork: network as StellarNetwork,
    networkPassphrase,
    adminAddress: env.ADMIN_ADDRESS?.trim() || undefined,
  };
}

export const config = loadConfig();

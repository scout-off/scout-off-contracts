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
  if (!(SUPPORTED_NETWORKS as readonly string[]).includes(network)) {
    errors.push(`STELLAR_NETWORK must be one of: ${SUPPORTED_NETWORKS.join(", ")}`);
  }

  const isTestnet = network === "testnet";

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

  return {
    jwtSecret,
    sep10SigningSecret,
    homeDomain,
    webAuthDomain,
    stellarNetwork: network as StellarNetwork,
    adminAddress,
  };
}

export const config = loadConfig();

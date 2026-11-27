import { loadConfig } from "../src/config";
import { generateKeyPairSync } from "crypto";
import { accessTokenTtlSeconds, refreshTokenTtlSeconds } from "../src/services/sep10";

const { privateKey } = generateKeyPairSync("ed25519");
const PRIVATE_PEM = privateKey
  .export({ type: "pkcs8", format: "pem" })
  .toString()
  // JWT_SIGNING_KEYS uses literal \n to fit in a single env-var line
  .replace(/\n/g, "\\n");

/** Minimal valid env using asymmetric keys (new default). */
const VALID_ASYMMETRIC = {
  JWT_SIGNING_KEYS: `key-1,${PRIVATE_PEM}`,
  SEP10_SIGNING_SECRET: "SCQAXBYFEN4M237TXXDUHGDHAGOO4M5E65RWCGCL5NXGCNK57CNRZF5M",
  HOME_DOMAIN: "scoutchain.test",
  WEB_AUTH_DOMAIN: "auth.scoutchain.test",
  STELLAR_NETWORK: "testnet",
};

const LOCAL_PASSPHRASE = "Standalone Network ; February 2017";
/** Minimal valid env using HS256 migration mode. */
const VALID_MIGRATION = {
  JWT_SECRET: "x".repeat(32),
  JWT_ALLOW_HS256_MIGRATION: "true",
  SEP10_SIGNING_SECRET: "SCQAXBYFEN4M237TXXDUHGDHAGOO4M5E65RWCGCL5NXGCNK57CNRZF5M",
  HOME_DOMAIN: "scoutchain.test",
  WEB_AUTH_DOMAIN: "auth.scoutchain.test",
  STELLAR_NETWORK: "testnet",
};

const VALID_ADMIN_ADDRESS = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

describe("loadConfig", () => {
  it("parses a valid asymmetric-key environment", () => {
    const config = loadConfig(VALID_ASYMMETRIC);
    expect(config.signingKeys).not.toBeNull();
    expect(config.signingKeys?.activeKid).toBe("key-1");
    expect(config.stellarNetwork).toBe("testnet");
    expect(config.allowHs256Migration).toBe(false);
  });

  it("parses a valid HS256 migration environment", () => {
    const config = loadConfig(VALID_MIGRATION);
    expect(config.jwtSecret).toBe(VALID_MIGRATION.JWT_SECRET);
    expect(config.allowHs256Migration).toBe(true);
    expect(config.signingKeys).toBeNull();
  });

  it("defaults STELLAR_NETWORK to testnet", () => {
    const { STELLAR_NETWORK: _, ...env } = VALID_ASYMMETRIC;
    expect(loadConfig(env).stellarNetwork).toBe("testnet");
  });

  it("defaults JWT_ISSUER to https://<HOME_DOMAIN>", () => {
    const config = loadConfig(VALID);
    expect(config.jwtIssuer).toBe(`https://${VALID.HOME_DOMAIN}`);
  });

  it("defaults JWT_AUDIENCE to scoutchain-api", () => {
    const config = loadConfig(VALID);
    expect(config.jwtAudience).toBe("scoutchain-api");
  });

  it("honours explicit JWT_ISSUER and JWT_AUDIENCE overrides", () => {
    const config = loadConfig({
      ...VALID,
      JWT_ISSUER: "https://issuer.example.com",
      JWT_AUDIENCE: "custom-audience",
    });
    expect(config.jwtIssuer).toBe("https://issuer.example.com");
    expect(config.jwtAudience).toBe("custom-audience");
  });

  it("lists missing required variables when both keys and migration are absent", () => {
    expect(() => loadConfig({})).toThrow(
      /SEP10_SIGNING_SECRET is required[\s\S]*HOME_DOMAIN is required[\s\S]*WEB_AUTH_DOMAIN is required/
    );
  });

  it("requires JWT_SECRET when JWT_ALLOW_HS256_MIGRATION=true", () => {
    expect(() =>
      loadConfig({ ...VALID_MIGRATION, JWT_SECRET: undefined })
    ).toThrow(/JWT_SECRET is required when JWT_ALLOW_HS256_MIGRATION/);
  });

  it("rejects a short JWT_SECRET in migration mode", () => {
    expect(() =>
      loadConfig({ ...VALID_MIGRATION, JWT_SECRET: "short" })
    ).toThrow(/at least 32 bytes/);
  });

  it("rejects an invalid SEP10_SIGNING_SECRET", () => {
    expect(() =>
      loadConfig({ ...VALID_ASYMMETRIC, SEP10_SIGNING_SECRET: "GABC" })
    ).toThrow(/SEP10_SIGNING_SECRET must be a valid/);
  });

  it("rejects an unsupported STELLAR_NETWORK", () => {
    expect(() =>
      loadConfig({ ...VALID_ASYMMETRIC, STELLAR_NETWORK: "devnet" })
    ).toThrow(/STELLAR_NETWORK must be one of/);
  });

  it("accepts STELLAR_NETWORK=local with NETWORK_PASSPHRASE", () => {
    const config = loadConfig({
      ...VALID,
      STELLAR_NETWORK: "local",
      NETWORK_PASSPHRASE: LOCAL_PASSPHRASE,
    });
    expect(config.stellarNetwork).toBe("local");
    expect(config.networkPassphrase).toBe(LOCAL_PASSPHRASE);
  });

  it("accepts STELLAR_NETWORK=custom with NETWORK_PASSPHRASE", () => {
    const config = loadConfig({
      ...VALID,
      STELLAR_NETWORK: "custom",
      NETWORK_PASSPHRASE: LOCAL_PASSPHRASE,
    });
    expect(config.stellarNetwork).toBe("custom");
    expect(config.networkPassphrase).toBe(LOCAL_PASSPHRASE);
  });

  it("rejects STELLAR_NETWORK=local without NETWORK_PASSPHRASE", () => {
    expect(() => loadConfig({ ...VALID, STELLAR_NETWORK: "local" })).toThrow(
      /NETWORK_PASSPHRASE is required/
    );
  });

  it("rejects STELLAR_NETWORK=local when NODE_ENV=production", () => {
    expect(() =>
      loadConfig({
        ...VALID,
        STELLAR_NETWORK: "local",
        NETWORK_PASSPHRASE: LOCAL_PASSPHRASE,
        NODE_ENV: "production",
      })
    ).toThrow(/STELLAR_NETWORK=local is not allowed when NODE_ENV=production/);
  });

  it("requires JWT_SIGNING_KEYS when not in migration mode", () => {
    expect(() =>
      loadConfig({
        SEP10_SIGNING_SECRET: VALID_ASYMMETRIC.SEP10_SIGNING_SECRET,
        HOME_DOMAIN: "scoutchain.test",
        WEB_AUTH_DOMAIN: "auth.scoutchain.test",
      })
    ).toThrow(/JWT_SIGNING_KEYS is required/);
  });

  describe("ADMIN_ADDRESS", () => {
    it("accepts a valid G-address", () => {
      const config = loadConfig({ ...VALID, ADMIN_ADDRESS: VALID_ADMIN_ADDRESS });
      expect(config.adminAddress).toBe(VALID_ADMIN_ADDRESS);
    });

    it("accepts an unset ADMIN_ADDRESS", () => {
      expect(() => loadConfig(VALID)).not.toThrow();
    });

    it("rejects an invalid string", () => {
      expect(() => loadConfig({ ...VALID, ADMIN_ADDRESS: "not-an-address" })).toThrow(
        /ADMIN_ADDRESS must be a valid/
      );
    });

    it("rejects a secret seed without echoing it", () => {
      const secret = VALID.SEP10_SIGNING_SECRET;
      let message = "";
      try {
        loadConfig({ ...VALID, ADMIN_ADDRESS: secret });
      } catch (err) {
        message = (err as Error).message;
      }
      expect(message).toMatch(/secret key/i);
      expect(message).not.toContain(secret);
    });
  });

  describe("HOME_DOMAIN and WEB_AUTH_DOMAIN", () => {
    it.each(["HOME_DOMAIN", "WEB_AUTH_DOMAIN"] as const)(
      "rejects a scheme in %s",
      (key) => {
        expect(() => loadConfig({ ...VALID, [key]: "https://scoutchain.test" })).toThrow(
          new RegExp(`${key} must be a hostname`)
        );
      }
    );

    it.each(["HOME_DOMAIN", "WEB_AUTH_DOMAIN"] as const)(
      "rejects a path in %s",
      (key) => {
        expect(() => loadConfig({ ...VALID, [key]: "scoutchain.test/path" })).toThrow(
          new RegExp(`${key} must be a hostname`)
        );
      }
    );

    it.each(["HOME_DOMAIN", "WEB_AUTH_DOMAIN"] as const)(
      "rejects whitespace in %s",
      (key) => {
        expect(() => loadConfig({ ...VALID, [key]: "scout chain.test" })).toThrow(
          new RegExp(`${key} must be a hostname`)
        );
      }
    );

    it("accepts localhost on testnet", () => {
      const config = loadConfig({
        ...VALID,
        HOME_DOMAIN: "localhost",
        WEB_AUTH_DOMAIN: "localhost",
      });
      expect(config.homeDomain).toBe("localhost");
      expect(config.webAuthDomain).toBe("localhost");
    });

    it("accepts host:port on testnet", () => {
      const config = loadConfig({
        ...VALID,
        HOME_DOMAIN: "localhost:8000",
        WEB_AUTH_DOMAIN: "localhost:8000",
      });
      expect(config.homeDomain).toBe("localhost:8000");
      expect(config.webAuthDomain).toBe("localhost:8000");
    });

    it("rejects host:port on mainnet", () => {
      expect(() =>
        loadConfig({
          ...VALID,
          STELLAR_NETWORK: "mainnet",
          HOME_DOMAIN: "scoutchain.test:443",
          WEB_AUTH_DOMAIN: "auth.scoutchain.test:443",
        })
      ).toThrow(/must be a hostname/);
    });
  });

  describe("token TTLs", () => {
    it("applies defaults when unset", () => {
      const config = loadConfig(VALID);
      expect(config.accessTokenTtlSeconds).toBe(900);
      expect(config.refreshTokenTtlSeconds).toBe(604800);
    });

    it("parses valid custom values", () => {
      const config = loadConfig({
        ...VALID,
        ACCESS_TOKEN_TTL_SECONDS: "300",
        REFRESH_TOKEN_TTL_SECONDS: "3600",
      });
      expect(config.accessTokenTtlSeconds).toBe(300);
      expect(config.refreshTokenTtlSeconds).toBe(3600);
    });

    it("rejects a non-numeric ACCESS_TOKEN_TTL_SECONDS", () => {
      expect(() =>
        loadConfig({ ...VALID, ACCESS_TOKEN_TTL_SECONDS: "15m" })
      ).toThrow(/ACCESS_TOKEN_TTL_SECONDS must be a positive integer/);
    });

    it("rejects a non-numeric REFRESH_TOKEN_TTL_SECONDS", () => {
      expect(() =>
        loadConfig({ ...VALID, REFRESH_TOKEN_TTL_SECONDS: "7d" })
      ).toThrow(/REFRESH_TOKEN_TTL_SECONDS must be a positive integer/);
    });

    it("rejects a zero ACCESS_TOKEN_TTL_SECONDS", () => {
      expect(() =>
        loadConfig({ ...VALID, ACCESS_TOKEN_TTL_SECONDS: "0" })
      ).toThrow(/ACCESS_TOKEN_TTL_SECONDS must be a positive integer/);
    });

    it("rejects a zero REFRESH_TOKEN_TTL_SECONDS", () => {
      expect(() =>
        loadConfig({ ...VALID, REFRESH_TOKEN_TTL_SECONDS: "0" })
      ).toThrow(/REFRESH_TOKEN_TTL_SECONDS must be a positive integer/);
    });

    it("rejects a negative ACCESS_TOKEN_TTL_SECONDS", () => {
      expect(() =>
        loadConfig({ ...VALID, ACCESS_TOKEN_TTL_SECONDS: "-60" })
      ).toThrow(/ACCESS_TOKEN_TTL_SECONDS must be a positive integer/);
    });

    it("rejects a negative REFRESH_TOKEN_TTL_SECONDS", () => {
      expect(() =>
        loadConfig({ ...VALID, REFRESH_TOKEN_TTL_SECONDS: "-60" })
      ).toThrow(/REFRESH_TOKEN_TTL_SECONDS must be a positive integer/);
    });

    it("rejects an ACCESS_TOKEN_TTL_SECONDS above the 15-minute ceiling", () => {
      expect(() =>
        loadConfig({ ...VALID, ACCESS_TOKEN_TTL_SECONDS: "901" })
      ).toThrow(/ACCESS_TOKEN_TTL_SECONDS must not exceed 900 seconds/);
    });

    it("accepts an ACCESS_TOKEN_TTL_SECONDS at the 15-minute ceiling", () => {
      const config = loadConfig({ ...VALID, ACCESS_TOKEN_TTL_SECONDS: "900" });
      expect(config.accessTokenTtlSeconds).toBe(900);
    });
  });
});

describe("accessTokenTtlSeconds", () => {
  const ORIGINAL_ENV = process.env;

  beforeEach(() => {
    process.env = { ...ORIGINAL_ENV };
    delete process.env.ACCESS_TOKEN_TTL_SECONDS;
  });

  afterEach(() => {
    process.env = ORIGINAL_ENV;
  });

  it("defaults to 900 when unset", () => {
    expect(accessTokenTtlSeconds()).toBe(900);
  });

  it("parses a valid value", () => {
    process.env.ACCESS_TOKEN_TTL_SECONDS = "60";
    expect(accessTokenTtlSeconds()).toBe(60);
  });

  it("clamps values above the 15-minute ceiling to 900", () => {
    process.env.ACCESS_TOKEN_TTL_SECONDS = "3600";
    expect(accessTokenTtlSeconds()).toBe(900);
  });

  it("falls back to the default for non-numeric values", () => {
    process.env.ACCESS_TOKEN_TTL_SECONDS = "abc";
    expect(accessTokenTtlSeconds()).toBe(900);
  });

  it("falls back to the default for zero", () => {
    process.env.ACCESS_TOKEN_TTL_SECONDS = "0";
    expect(accessTokenTtlSeconds()).toBe(900);
  });

  it("falls back to the default for negative values", () => {
    process.env.ACCESS_TOKEN_TTL_SECONDS = "-5";
    expect(accessTokenTtlSeconds()).toBe(900);
  });

  it("floors fractional values", () => {
    process.env.ACCESS_TOKEN_TTL_SECONDS = "90.7";
    expect(accessTokenTtlSeconds()).toBe(90);
  });
});

describe("refreshTokenTtlSeconds", () => {
  const ORIGINAL_ENV = process.env;

  beforeEach(() => {
    process.env = { ...ORIGINAL_ENV };
    delete process.env.REFRESH_TOKEN_TTL_SECONDS;
  });

  afterEach(() => {
    process.env = ORIGINAL_ENV;
  });

  it("defaults to 604800 when unset", () => {
    expect(refreshTokenTtlSeconds()).toBe(604800);
  });

  it("parses a valid value", () => {
    process.env.REFRESH_TOKEN_TTL_SECONDS = "60";
    expect(refreshTokenTtlSeconds()).toBe(60);
  });

  it("falls back to the default for non-numeric values", () => {
    process.env.REFRESH_TOKEN_TTL_SECONDS = "abc";
    expect(refreshTokenTtlSeconds()).toBe(604800);
  });

  it("falls back to the default for zero", () => {
    process.env.REFRESH_TOKEN_TTL_SECONDS = "0";
    expect(refreshTokenTtlSeconds()).toBe(604800);
  });

  it("falls back to the default for negative values", () => {
    process.env.REFRESH_TOKEN_TTL_SECONDS = "-5";
    expect(refreshTokenTtlSeconds()).toBe(604800);
  });

  it("floors fractional values", () => {
    process.env.REFRESH_TOKEN_TTL_SECONDS = "90.7";
    expect(refreshTokenTtlSeconds()).toBe(90);
  });
});
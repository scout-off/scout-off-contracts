import { loadConfig } from "../src/config";

const VALID = {
  JWT_SECRET: "x".repeat(32),
  SEP10_SIGNING_SECRET: "SCQAXBYFEN4M237TXXDUHGDHAGOO4M5E65RWCGCL5NXGCNK57CNRZF5M",
  HOME_DOMAIN: "scoutchain.test",
  WEB_AUTH_DOMAIN: "auth.scoutchain.test",
  STELLAR_NETWORK: "testnet",
};

const VALID_ADMIN_ADDRESS = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

describe("loadConfig", () => {
  it("parses a valid environment", () => {
    const config = loadConfig(VALID);
    expect(config.jwtSecret).toBe(VALID.JWT_SECRET);
    expect(config.stellarNetwork).toBe("testnet");
  });

  it("defaults STELLAR_NETWORK to testnet", () => {
    const { STELLAR_NETWORK, ...env } = VALID;
    expect(loadConfig(env).stellarNetwork).toBe("testnet");
  });

  it("lists every missing variable", () => {
    expect(() => loadConfig({})).toThrow(
      /JWT_SECRET is required[\s\S]*SEP10_SIGNING_SECRET is required[\s\S]*HOME_DOMAIN is required[\s\S]*WEB_AUTH_DOMAIN is required/
    );
  });

  it("rejects a short JWT_SECRET", () => {
    expect(() => loadConfig({ ...VALID, JWT_SECRET: "short" })).toThrow(/at least 32 bytes/);
  });

  it("rejects an invalid SEP10_SIGNING_SECRET", () => {
    expect(() => loadConfig({ ...VALID, SEP10_SIGNING_SECRET: "GABC" })).toThrow(
      /SEP10_SIGNING_SECRET must be a valid/
    );
  });

  it("rejects an unsupported STELLAR_NETWORK", () => {
    expect(() => loadConfig({ ...VALID, STELLAR_NETWORK: "devnet" })).toThrow(
      /STELLAR_NETWORK must be one of/
    );
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
});
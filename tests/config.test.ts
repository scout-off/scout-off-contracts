import { loadConfig } from "../src/config";

const VALID = {
  JWT_SECRET: "x".repeat(32),
  SEP10_SIGNING_SECRET: "SCQAXBYFEN4M237TXXDUHGDHAGOO4M5E65RWCGCL5NXGCNK57CNRZF5M",
  HOME_DOMAIN: "scoutchain.test",
  WEB_AUTH_DOMAIN: "auth.scoutchain.test",
  STELLAR_NETWORK: "testnet",
};

const LOCAL_PASSPHRASE = "Standalone Network ; February 2017";

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
    ).toThrow(/local network is not allowed in production/);
  });
});
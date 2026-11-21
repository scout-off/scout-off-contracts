import { loadConfig } from "../src/config";

const VALID = {
  JWT_SECRET: "x".repeat(32),
  SEP10_SIGNING_SECRET: "SCQAXBYFEN4M237TXXDUHGDHAGOO4M5E65RWCGCL5NXGCNK57CNRZF5M",
  HOME_DOMAIN: "scoutchain.test",
  WEB_AUTH_DOMAIN: "auth.scoutchain.test",
  STELLAR_NETWORK: "testnet",
};

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

  it("defaults PORT to 3000 when unset", () => {
    expect(loadConfig(VALID).port).toBe(3000);
  });

  it("parses a valid PORT", () => {
    expect(loadConfig({ ...VALID, PORT: "8080" }).port).toBe(8080);
  });

  it("rejects a non-integer PORT", () => {
    expect(() => loadConfig({ ...VALID, PORT: "abc" })).toThrow(/PORT must be an integer/);
  });

  it("rejects an out-of-range PORT above 65535", () => {
    expect(() => loadConfig({ ...VALID, PORT: "99999" })).toThrow(/PORT must be an integer/);
  });

  it("rejects an out-of-range PORT of 0", () => {
    expect(() => loadConfig({ ...VALID, PORT: "0" })).toThrow(/PORT must be an integer/);
  });
});
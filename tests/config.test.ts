import { loadConfig } from "../src/config";
import { accessTokenTtlSeconds, refreshTokenTtlSeconds } from "../src/config";

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
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
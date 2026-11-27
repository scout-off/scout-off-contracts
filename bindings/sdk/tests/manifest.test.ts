/**
 * Unit tests for manifest parsing.
 */
import { parseManifest } from "../src/manifest";

const VALID_CAMEL = {
  registrationContractId: "CREGID",
  verificationContractId: "CVERID",
  progressContractId: "CPROGID",
  scoutAccessContractId: "CSCOUTID",
  networkPassphrase: "Test SDF Network ; September 2015",
};

const VALID_SNAKE = {
  REGISTRATION_CONTRACT_ID: "CREGID",
  VERIFICATION_CONTRACT_ID: "CVERID",
  PROGRESS_CONTRACT_ID: "CPROGID",
  SCOUT_ACCESS_CONTRACT_ID: "CSCOUTID",
  NETWORK_PASSPHRASE: "Test SDF Network ; September 2015",
};

describe("parseManifest", () => {
  it("parses camelCase keys", () => {
    const m = parseManifest(VALID_CAMEL);
    expect(m.registrationContractId).toBe("CREGID");
    expect(m.networkPassphrase).toBe("Test SDF Network ; September 2015");
  });

  it("parses UPPER_SNAKE env-var keys", () => {
    const m = parseManifest(VALID_SNAKE);
    expect(m.scoutAccessContractId).toBe("CSCOUTID");
  });

  it("camelCase takes precedence over UPPER_SNAKE", () => {
    const m = parseManifest({
      ...VALID_SNAKE,
      registrationContractId: "CAMEL_WINS",
    });
    expect(m.registrationContractId).toBe("CAMEL_WINS");
  });

  it("throws for a missing required field", () => {
    const { registrationContractId: _, ...incomplete } = VALID_CAMEL;
    expect(() => parseManifest(incomplete)).toThrow(/registrationContractId/);
  });
});

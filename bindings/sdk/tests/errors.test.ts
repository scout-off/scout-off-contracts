/**
 * Unit tests for @scoutchain/sdk typed error classes and unwrapResult.
 */
import {
  ScoutChainContractError,
  ProgressContractError,
  RegistrationContractError,
  ScoutAccessContractError,
  VerificationContractError,
  ProgressError,
  RegistrationError,
  ScoutAccessError,
  VerificationError,
} from "../src/errors";
import { unwrapResult, errorFactories } from "../src/result";

describe("ScoutChainContractError", () => {
  it("constructs with the right properties", () => {
    const err = new ProgressContractError(7, "PlayerNotFound");
    expect(err.code).toBe(7);
    expect(err.variant).toBe("PlayerNotFound");
    expect(err.contract).toBe("Progress");
    expect(err.message).toBe("Progress error 7: PlayerNotFound");
    expect(err.name).toBe("ProgressContractError");
    expect(err).toBeInstanceOf(ScoutChainContractError);
    expect(err).toBeInstanceOf(ProgressContractError);
    expect(err).toBeInstanceOf(Error);
  });

  it("instanceof works across all four contract error classes", () => {
    expect(new RegistrationContractError(1, "X")).toBeInstanceOf(RegistrationContractError);
    expect(new ScoutAccessContractError(1, "X")).toBeInstanceOf(ScoutAccessContractError);
    expect(new VerificationContractError(1, "X")).toBeInstanceOf(VerificationContractError);
  });
});

describe("ProgressError namespace", () => {
  it("fromCode returns the right variant name", () => {
    const err = ProgressError.fromCode(7);
    expect(err.variant).toBe("PlayerNotFound");
    expect(err.code).toBe(7);
  });

  it("fromCode returns Unknown for unrecognised codes", () => {
    const err = ProgressError.fromCode(9999);
    expect(err.variant).toBe("Unknown(9999)");
  });

  it("constants match the Rust source codes", () => {
    expect(ProgressError.AlreadyInitialized).toBe(1);
    expect(ProgressError.ContractPaused).toBe(3);
    expect(ProgressError.HistoryRootMissing).toBe(28);
  });
});

describe("RegistrationError namespace", () => {
  it("fromCode maps codes correctly", () => {
    expect(RegistrationError.fromCode(3).variant).toBe("PlayerNotFound");
    expect(RegistrationError.fromCode(16).variant).toBe("RegistrationCooldown");
  });
});

describe("ScoutAccessError namespace", () => {
  it("fromCode maps ProContactLimitReached", () => {
    const err = ScoutAccessError.fromCode(20);
    expect(err.variant).toBe("ProContactLimitReached");
    expect(err).toBeInstanceOf(ScoutAccessContractError);
  });

  it("constants are correct", () => {
    expect(ScoutAccessError.ProContactLimitReached).toBe(20);
    expect(ScoutAccessError.ContactNotFound).toBe(57);
    expect(ScoutAccessError.InvalidTrialIndex).toBe(58);
  });
});

describe("VerificationError namespace", () => {
  it("fromCode maps BatchTooLarge", () => {
    const err = VerificationError.fromCode(64);
    expect(err.variant).toBe("BatchTooLarge");
  });
});

describe("unwrapResult", () => {
  it("returns plain values directly (no isErr)", () => {
    expect(unwrapResult("hello", errorFactories.progress)).toBe("hello");
    expect(unwrapResult(42n, errorFactories.progress)).toBe(42n);
    expect(unwrapResult(null, errorFactories.progress)).toBeNull();
  });

  it("unwraps a successful Result-like value", () => {
    const result = {
      isErr: () => false,
      unwrap: () => "success",
      unwrapErr: () => null,
    };
    expect(unwrapResult(result, errorFactories.registration)).toBe("success");
  });

  it("throws a typed error for a failed Result with numeric code", () => {
    const result = {
      isErr: () => true,
      unwrap: () => null,
      unwrapErr: () => 7,
    };
    expect(() =>
      unwrapResult(result, errorFactories.progress, "test")
    ).toThrow(ProgressContractError);
  });

  it("extracts error code from Error(Contract, #N) message string", () => {
    const result = {
      isErr: () => true,
      unwrap: () => null,
      unwrapErr: () => ({ message: "Error(Contract, #20)" }),
    };
    let thrown: unknown;
    try {
      unwrapResult(result, errorFactories.scoutAccess, "test");
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(ScoutAccessContractError);
    expect((thrown as ScoutAccessContractError).code).toBe(20);
    expect((thrown as ScoutAccessContractError).variant).toBe("ProContactLimitReached");
  });

  it("throws a generic Error when code cannot be extracted", () => {
    const result = {
      isErr: () => true,
      unwrap: () => null,
      unwrapErr: () => "some opaque error",
    };
    expect(() =>
      unwrapResult(result, errorFactories.verification, "myop")
    ).toThrow(/myop.*contract returned error/i);
  });
});

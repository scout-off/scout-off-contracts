/**
 * Unit tests for ScoutChain high-level helpers with fully mocked RPC clients.
 */
import { ScoutChain } from "../src/ScoutChain";
import {
  ScoutAccessContractError,
  ScoutAccessError,
  ProgressContractError,
  RegistrationContractError,
} from "../src/errors";

const MANIFEST = {
  registrationContractId: "CREGID",
  verificationContractId: "CVERID",
  progressContractId: "CPROGID",
  scoutAccessContractId: "CSCOUTID",
  networkPassphrase: "Test SDF Network ; September 2015",
};

const OPTS = { rpcUrl: "http://localhost:8000" };

/** Build a ScoutChain instance with its client factories monkey-patched. */
function sdkWithMocks(overrides: {
  registration?: object;
  verification?: object;
  progress?: object;
  scoutAccess?: object;
}): ScoutChain {
  const sdk = ScoutChain.fromManifest(MANIFEST, OPTS);
  if (overrides.registration) {
    (sdk as unknown as { registrationClient: () => Promise<unknown> }).registrationClient =
      async () => overrides.registration;
  }
  if (overrides.verification) {
    (sdk as unknown as { verificationClient: () => Promise<unknown> }).verificationClient =
      async () => overrides.verification;
  }
  if (overrides.progress) {
    (sdk as unknown as { progressClient: () => Promise<unknown> }).progressClient =
      async () => overrides.progress;
  }
  if (overrides.scoutAccess) {
    (sdk as unknown as { scoutAccessClient: () => Promise<unknown> }).scoutAccessClient =
      async () => overrides.scoutAccess;
  }
  return sdk;
}

// ─── ScoutChain.fromManifest ──────────────────────────────────────────────────

describe("ScoutChain.fromManifest", () => {
  it("stores the manifest and rpcUrl", () => {
    const sdk = ScoutChain.fromManifest(MANIFEST, OPTS);
    expect(sdk.manifest.registrationContractId).toBe("CREGID");
    expect(sdk.rpcUrl).toBe("http://localhost:8000");
  });

  it("accepts UPPER_SNAKE manifest keys", () => {
    const sdk = ScoutChain.fromManifest(
      {
        REGISTRATION_CONTRACT_ID: "CR",
        VERIFICATION_CONTRACT_ID: "CV",
        PROGRESS_CONTRACT_ID: "CP",
        SCOUT_ACCESS_CONTRACT_ID: "CS",
        NETWORK_PASSPHRASE: "Test SDF Network ; September 2015",
      },
      OPTS,
    );
    expect(sdk.manifest.registrationContractId).toBe("CR");
  });
});

// ─── registerPlayer ───────────────────────────────────────────────────────────

describe("registerPlayer", () => {
  it("calls register_player and resolves on success", async () => {
    const register_player = jest.fn().mockResolvedValue("ok");
    const sdk = sdkWithMocks({ registration: { register_player } });
    await sdk.registerPlayer({ wallet: "GWALLET" });
    expect(register_player).toHaveBeenCalledWith({
      wallet: "GWALLET",
      metadata_uri: "",
    });
  });

  it("throws RegistrationContractError on contract failure", async () => {
    const register_player = jest.fn().mockResolvedValue({
      isErr: () => true,
      unwrapErr: () => ({ message: "Error(Contract, #3)" }),
      unwrap: () => null,
    });
    const sdk = sdkWithMocks({ registration: { register_player } });
    await expect(sdk.registerPlayer({ wallet: "G" })).rejects.toBeInstanceOf(
      RegistrationContractError,
    );
  });
});

// ─── subscribe ────────────────────────────────────────────────────────────────

describe("subscribe", () => {
  it("resolves on success", async () => {
    const subscribe = jest.fn().mockResolvedValue("ok");
    const sdk = sdkWithMocks({ scoutAccess: { subscribe } });
    await sdk.subscribe({ scout: "GSCOUT", tier: "Pro" });
    expect(subscribe).toHaveBeenCalledWith({ scout: "GSCOUT", tier: "Pro" });
  });

  it("throws ScoutAccessContractError with correct code on failure", async () => {
    const subscribe = jest.fn().mockResolvedValue({
      isErr: () => true,
      unwrapErr: () => 20,
      unwrap: () => null,
    });
    const sdk = sdkWithMocks({ scoutAccess: { subscribe } });
    let thrown: unknown;
    try {
      await sdk.subscribe({ scout: "G", tier: "Basic" });
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(ScoutAccessContractError);
    expect((thrown as ScoutAccessContractError).code).toBe(
      ScoutAccessError.ProContactLimitReached,
    );
  });
});

// ─── payToContact ─────────────────────────────────────────────────────────────

describe("payToContact", () => {
  it("resolves on success", async () => {
    const pay_to_contact = jest.fn().mockResolvedValue("ok");
    const sdk = sdkWithMocks({ scoutAccess: { pay_to_contact } });
    await sdk.payToContact({ scout: "GSCOUT", playerId: 42n });
    expect(pay_to_contact).toHaveBeenCalledWith({ scout: "GSCOUT", player_id: 42n });
  });
});

// ─── getPlayerHistory ─────────────────────────────────────────────────────────

describe("getPlayerHistory", () => {
  it("returns current level and paginated history", async () => {
    const page1: object[] = Array.from({ length: 50 }, (_, i) => ({
      player_id: 1n,
      old_level: "Unverified",
      new_level: "VerifiedIdentity",
      updated_by: "G",
      updated_at: BigInt(1700000000 + i),
      milestone_ref: i,
      ledger_sequence: 1000 + i,
    }));
    const page2 = page1.slice(0, 10);

    const get_level = jest.fn().mockResolvedValue("VerifiedIdentity");
    const get_history_page_with_cursor = jest
      .fn()
      .mockResolvedValueOnce(page1)
      .mockResolvedValueOnce(page2);

    const sdk = sdkWithMocks({
      progress: { get_level, get_history_page_with_cursor },
    });

    const result = await sdk.getPlayerHistory({ playerId: 1n, pageSize: 50 });
    expect(result.currentLevel).toBe("VerifiedIdentity");
    expect(result.history).toHaveLength(60);
    expect(get_history_page_with_cursor).toHaveBeenCalledTimes(2);
  });

  it("throws ProgressContractError when get_level returns a contract error", async () => {
    const get_level = jest.fn().mockResolvedValue({
      isErr: () => true,
      unwrapErr: () => 7,
      unwrap: () => null,
    });
    const sdk = sdkWithMocks({ progress: { get_level } });
    await expect(sdk.getPlayerHistory({ playerId: 1n })).rejects.toBeInstanceOf(
      ProgressContractError,
    );
  });

  it("returns empty history when no pages returned", async () => {
    const get_level = jest.fn().mockResolvedValue("Unverified");
    const get_history_page_with_cursor = jest.fn().mockResolvedValue([]);
    const sdk = sdkWithMocks({ progress: { get_level, get_history_page_with_cursor } });
    const result = await sdk.getPlayerHistory({ playerId: 99n });
    expect(result.currentLevel).toBe("Unverified");
    expect(result.history).toHaveLength(0);
  });
});

// ─── logTrialOffer / confirmTrialOffer ────────────────────────────────────────

describe("logTrialOffer / confirmTrialOffer", () => {
  it("logTrialOffer calls the contract with correct args", async () => {
    const log_trial_offer = jest.fn().mockResolvedValue("ok");
    const sdk = sdkWithMocks({ scoutAccess: { log_trial_offer } });
    await sdk.logTrialOffer({ scout: "GS", playerId: 5n, offerDetails: "offer" });
    expect(log_trial_offer).toHaveBeenCalledWith({
      scout: "GS",
      player_id: 5n,
      offer_details: "offer",
    });
  });

  it("confirmTrialOffer calls the contract with correct args", async () => {
    const confirm_trial_offer = jest.fn().mockResolvedValue("ok");
    const sdk = sdkWithMocks({ scoutAccess: { confirm_trial_offer } });
    await sdk.confirmTrialOffer({ player: "GP", offerIndex: 3 });
    expect(confirm_trial_offer).toHaveBeenCalledWith({ player: "GP", offer_index: 3 });
  });
});

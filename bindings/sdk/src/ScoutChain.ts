/**
 * @scoutchain/sdk – unified client composing all four contract bindings.
 *
 * Usage:
 *   const sdk = ScoutChain.fromManifest(manifest, { rpcUrl, signTransaction });
 *   await sdk.registerPlayer({ wallet, metadataUri });
 *   await sdk.subscribe({ scout, tier });
 *   const history = await sdk.getPlayerHistory({ playerId });
 */

// The generated binding packages do not ship compiled artefacts in this repo
// (they are peer packages listed in package.json).  The SDK imports their
// types at compile time and their constructors at runtime.  We use dynamic
// imports so that tree-shakers can drop unused contracts and so the SDK builds
// even if only some bindings are installed.

// The generated binding packages are peer dependencies installed separately.
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore – peer dep, not installed in this package's node_modules
import type { Client as RegistrationClient } from "@scoutchain/bindings-registration";
// @ts-ignore
import type { Client as VerificationClient } from "@scoutchain/bindings-verification";
// @ts-ignore
import type { Client as ProgressClient } from "@scoutchain/bindings-progress";
// @ts-ignore
import type { Client as ScoutAccessClient } from "@scoutchain/bindings-scout-access";

import { ContractManifest, parseManifest } from "./manifest";
import { unwrapResult, errorFactories } from "./result";
import {
  ScoutChainContractError,
  ProgressContractError,
  RegistrationContractError,
  ScoutAccessContractError,
  VerificationContractError,
} from "./errors";

/** Minimal sign-transaction callback matching the Stellar SDK convention. */
export type SignTransaction = (
  xdr: string,
  opts?: { networkPassphrase?: string },
) => Promise<string>;

/** Options passed to `ScoutChain.fromManifest`. */
export interface ScoutChainOptions {
  rpcUrl: string;
  /** Optional signer callback – required only for state-changing operations. */
  signTransaction?: SignTransaction;
}

// ─── High-level helper parameter / result types ───────────────────────────────

export interface RegisterPlayerParams {
  wallet: string;
  metadataUri?: string;
}

export interface ApproveMilestoneParams {
  validator: string;
  playerId: bigint;
  description: string;
  evidence: string;
  specializationId?: number;
}

export interface SubscribeParams {
  scout: string;
  /** "Basic" | "Pro" | "Elite" */
  tier: string;
}

export interface PayToContactParams {
  scout: string;
  playerId: bigint;
}

export interface LogTrialOfferParams {
  scout: string;
  playerId: bigint;
  offerDetails: string;
}

export interface ConfirmTrialOfferParams {
  player: string;
  offerIndex: number;
}

export interface ProgressEntry {
  player_id: bigint;
  old_level: string;
  new_level: string;
  updated_by: string;
  updated_at: bigint;
  milestone_ref: number;
  ledger_sequence: number;
}

export interface GetPlayerHistoryResult {
  currentLevel: string;
  history: ProgressEntry[];
}

// ─── ScoutChain ───────────────────────────────────────────────────────────────

export class ScoutChain {
  private constructor(
    public readonly manifest: ContractManifest,
    public readonly rpcUrl: string,
    private readonly _signTransaction: SignTransaction | undefined,
  ) {}

  /**
   * Create an SDK instance from a deployment manifest.
   *
   * @param manifest  Contract IDs + network passphrase. Accepts the
   *                  camelCase object shape or the env-var UPPER_SNAKE shape.
   * @param opts      `rpcUrl` (required), optional `signTransaction` signer.
   */
  static fromManifest(
    manifest: ContractManifest | Record<string, string>,
    opts: ScoutChainOptions,
  ): ScoutChain {
    const parsed =
      "registrationContractId" in manifest
        ? (manifest as ContractManifest)
        : parseManifest(manifest as Record<string, string>);
    return new ScoutChain(parsed, opts.rpcUrl, opts.signTransaction);
  }

  // ── Low-level client accessors ─────────────────────────────────────────────

  /** Lazily-loaded raw generated Registration client. */
  async registrationClient(): Promise<RegistrationClient> {
    // eslint-disable-next-line @typescript-eslint/ban-ts-comment
    // @ts-ignore – peer dep
    const { Client } = await import("@scoutchain/bindings-registration");
    return new (Client as new (opts: object) => RegistrationClient)({
      contractId: this.manifest.registrationContractId,
      networkPassphrase: this.manifest.networkPassphrase,
      rpcUrl: this.rpcUrl,
      signTransaction: this._signTransaction,
    });
  }

  /** Lazily-loaded raw generated Verification client. */
  async verificationClient(): Promise<VerificationClient> {
    // @ts-ignore – peer dep
    const { Client } = await import("@scoutchain/bindings-verification");
    return new (Client as new (opts: object) => VerificationClient)({
      contractId: this.manifest.verificationContractId,
      networkPassphrase: this.manifest.networkPassphrase,
      rpcUrl: this.rpcUrl,
      signTransaction: this._signTransaction,
    });
  }

  /** Lazily-loaded raw generated Progress client. */
  async progressClient(): Promise<ProgressClient> {
    // @ts-ignore – peer dep
    const { Client } = await import("@scoutchain/bindings-progress");
    return new (Client as new (opts: object) => ProgressClient)({
      contractId: this.manifest.progressContractId,
      networkPassphrase: this.manifest.networkPassphrase,
      rpcUrl: this.rpcUrl,
      signTransaction: this._signTransaction,
    });
  }

  /** Lazily-loaded raw generated ScoutAccess client. */
  async scoutAccessClient(): Promise<ScoutAccessClient> {
    // @ts-ignore – peer dep
    const { Client } = await import("@scoutchain/bindings-scout-access");
    return new (Client as new (opts: object) => ScoutAccessClient)({
      contractId: this.manifest.scoutAccessContractId,
      networkPassphrase: this.manifest.networkPassphrase,
      rpcUrl: this.rpcUrl,
      signTransaction: this._signTransaction,
    });
  }

  // ── High-level helpers ──────────────────────────────────────────────────────

  /**
   * Register a new player on-chain.
   * Throws `RegistrationContractError` on contract error.
   */
  async registerPlayer(params: RegisterPlayerParams): Promise<void> {
    const client = await this.registrationClient();
    const assembled = await (client as unknown as {
      register_player: (args: { wallet: string; metadata_uri?: string }) => Promise<unknown>;
    }).register_player({
      wallet: params.wallet,
      metadata_uri: params.metadataUri ?? "",
    });
    unwrapResult(assembled, errorFactories.registration, "registerPlayer");
  }

  /**
   * Approve a milestone for a player (validator action).
   * Throws `VerificationContractError` on contract error.
   */
  async approveMilestone(params: ApproveMilestoneParams): Promise<void> {
    const client = await this.verificationClient();
    const assembled = await (client as unknown as {
      approve_milestone: (args: {
        validator: string;
        player_id: bigint;
        description: string;
        evidence: string;
        specialization_id?: number;
      }) => Promise<unknown>;
    }).approve_milestone({
      validator: params.validator,
      player_id: params.playerId,
      description: params.description,
      evidence: params.evidence,
      specialization_id: params.specializationId,
    });
    unwrapResult(assembled, errorFactories.verification, "approveMilestone");
  }

  /**
   * Subscribe a scout to a tier.
   * Throws `ScoutAccessContractError` on contract error.
   */
  async subscribe(params: SubscribeParams): Promise<void> {
    const client = await this.scoutAccessClient();
    const assembled = await (client as unknown as {
      subscribe: (args: { scout: string; tier: string }) => Promise<unknown>;
    }).subscribe({
      scout: params.scout,
      tier: params.tier,
    });
    unwrapResult(assembled, errorFactories.scoutAccess, "subscribe");
  }

  /**
   * Pay to unlock a player's contact details.
   * Throws `ScoutAccessContractError` on contract error.
   */
  async payToContact(params: PayToContactParams): Promise<void> {
    const client = await this.scoutAccessClient();
    const assembled = await (client as unknown as {
      pay_to_contact: (args: { scout: string; player_id: bigint }) => Promise<unknown>;
    }).pay_to_contact({
      scout: params.scout,
      player_id: params.playerId,
    });
    unwrapResult(assembled, errorFactories.scoutAccess, "payToContact");
  }

  /**
   * Log a trial offer from a scout to a player.
   * Throws `ScoutAccessContractError` on contract error.
   */
  async logTrialOffer(params: LogTrialOfferParams): Promise<void> {
    const client = await this.scoutAccessClient();
    const assembled = await (client as unknown as {
      log_trial_offer: (args: {
        scout: string;
        player_id: bigint;
        offer_details: string;
      }) => Promise<unknown>;
    }).log_trial_offer({
      scout: params.scout,
      player_id: params.playerId,
      offer_details: params.offerDetails,
    });
    unwrapResult(assembled, errorFactories.scoutAccess, "logTrialOffer");
  }

  /**
   * Confirm a trial offer (player action).
   * Throws `ScoutAccessContractError` on contract error.
   */
  async confirmTrialOffer(params: ConfirmTrialOfferParams): Promise<void> {
    const client = await this.scoutAccessClient();
    const assembled = await (client as unknown as {
      confirm_trial_offer: (args: {
        player: string;
        offer_index: number;
      }) => Promise<unknown>;
    }).confirm_trial_offer({
      player: params.player,
      offer_index: params.offerIndex,
    });
    unwrapResult(assembled, errorFactories.scoutAccess, "confirmTrialOffer");
  }

  /**
   * Get a player's current level and full history.
   * Paginates internally until all entries have been fetched.
   * Throws `ProgressContractError` on contract error.
   */
  async getPlayerHistory(params: {
    playerId: bigint;
    pageSize?: number;
  }): Promise<GetPlayerHistoryResult> {
    const client = await this.progressClient();
    const { playerId, pageSize = 50 } = params;

    const levelAssembled = await (client as unknown as {
      get_level: (args: { player_id: bigint }) => Promise<unknown>;
    }).get_level({ player_id: playerId });
    const currentLevel = String(
      unwrapResult(levelAssembled, errorFactories.progress, "get_level"),
    );

    // Paginate with get_history_page_with_cursor until exhausted.
    const history: ProgressEntry[] = [];
    let cursor: bigint | undefined = undefined;
    for (;;) {
      const pageAssembled = await (client as unknown as {
        get_history_page_with_cursor: (args: {
          player_id: bigint;
          cursor?: bigint;
          limit: number;
        }) => Promise<unknown>;
      }).get_history_page_with_cursor({
        player_id: playerId,
        cursor,
        limit: pageSize,
      });
      const page = unwrapResult<ProgressEntry[]>(
        pageAssembled,
        errorFactories.progress,
        "get_history_page_with_cursor",
      );
      if (!page || page.length === 0) break;
      history.push(...page);
      if (page.length < pageSize) break;
      cursor = page[page.length - 1].updated_at;
    }

    return { currentLevel, history };
  }
}

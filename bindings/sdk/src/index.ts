/**
 * @scoutchain/sdk – unified ScoutChain client.
 *
 * @example
 * ```ts
 * import { ScoutChain } from "@scoutchain/sdk";
 * import { ScoutAccessError } from "@scoutchain/sdk/errors";
 *
 * const sdk = ScoutChain.fromManifest(
 *   {
 *     registrationContractId: "C...",
 *     verificationContractId: "C...",
 *     progressContractId: "C...",
 *     scoutAccessContractId: "C...",
 *     networkPassphrase: "Test SDF Network ; September 2015",
 *   },
 *   { rpcUrl: "https://soroban-testnet.stellar.org", signTransaction }
 * );
 *
 * // High-level helper
 * const history = await sdk.getPlayerHistory({ playerId: 1n });
 *
 * // Typed errors
 * try {
 *   await sdk.subscribe({ scout: "G...", tier: "Basic" });
 * } catch (err) {
 *   if (err instanceof ScoutAccessContractError &&
 *       err.code === ScoutAccessError.SubscriptionDowngradeNotAllowed) {
 *     // handle
 *   }
 * }
 * ```
 */
export { ScoutChain } from "./ScoutChain";
export type {
  ScoutChainOptions,
  SignTransaction,
  RegisterPlayerParams,
  ApproveMilestoneParams,
  SubscribeParams,
  PayToContactParams,
  LogTrialOfferParams,
  ConfirmTrialOfferParams,
  GetPlayerHistoryResult,
  ProgressEntry,
} from "./ScoutChain";

export { parseManifest } from "./manifest";
export type { ContractManifest } from "./manifest";

export {
  unwrapResult,
  errorFactories,
  ScoutChainContractError,
  ProgressContractError,
  RegistrationContractError,
  ScoutAccessContractError,
  VerificationContractError,
} from "./result";

export {
  ProgressError,
  RegistrationError,
  ScoutAccessError,
  VerificationError,
} from "./errors";

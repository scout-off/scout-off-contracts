/**
 * Deployment manifest shape – mirrors the .env.contracts.json convention used
 * across the ScoutChain repo.
 */
export interface ContractManifest {
  /** Contract ID of the registration contract. */
  registrationContractId: string;
  /** Contract ID of the verification contract. */
  verificationContractId: string;
  /** Contract ID of the progress contract. */
  progressContractId: string;
  /** Contract ID of the scout_access contract. */
  scoutAccessContractId: string;
  /** Stellar network passphrase, e.g. "Test SDF Network ; September 2015". */
  networkPassphrase: string;
}

/**
 * Load a manifest from an env-contracts-json style object.
 *
 * Accepts either the flat { REGISTRATION_CONTRACT_ID, ... } shape produced
 * by the deployment scripts, or a pre-parsed camelCase object.
 */
export function parseManifest(raw: Record<string, string>): ContractManifest {
  // Support both UPPER_SNAKE env-var style and camelCase.
  const get = (camel: string, snake: string): string => {
    const v = raw[camel] ?? raw[snake];
    if (!v) throw new Error(`manifest is missing "${camel}" / "${snake}"`);
    return v;
  };

  return {
    registrationContractId: get("registrationContractId", "REGISTRATION_CONTRACT_ID"),
    verificationContractId: get("verificationContractId", "VERIFICATION_CONTRACT_ID"),
    progressContractId: get("progressContractId", "PROGRESS_CONTRACT_ID"),
    scoutAccessContractId: get("scoutAccessContractId", "SCOUT_ACCESS_CONTRACT_ID"),
    networkPassphrase: get("networkPassphrase", "NETWORK_PASSPHRASE"),
  };
}

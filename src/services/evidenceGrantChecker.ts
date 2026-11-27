import {
  Account,
  Address,
  BASE_FEE,
  Contract,
  Networks,
  TransactionBuilder,
  nativeToScVal,
  rpc,
  scValToNative,
} from "@stellar/stellar-sdk";

export interface EvidenceGrantChecker {
  hasEvidenceAccess(playerId: string, scout: string): Promise<boolean>;
}

export interface SorobanGrantCheckerConfig {
  rpcUrl: string;
  contractId: string;
  networkPassphrase: string;
}

export function grantCheckerConfigFromEnv(): SorobanGrantCheckerConfig {
  const { SOROBAN_RPC_URL, SCOUT_ACCESS_CONTRACT_ID } = process.env;
  if (!SOROBAN_RPC_URL || !SCOUT_ACCESS_CONTRACT_ID) {
    throw new Error("Soroban RPC and scout_access contract configuration is required");
  }
  const networkPassphrase =
    process.env.STELLAR_NETWORK === "mainnet" ? Networks.PUBLIC : Networks.TESTNET;
  return {
    rpcUrl: SOROBAN_RPC_URL,
    contractId: SCOUT_ACCESS_CONTRACT_ID,
    networkPassphrase,
  };
}

export function createSorobanGrantChecker(
  config: SorobanGrantCheckerConfig
): EvidenceGrantChecker {
  const server = new rpc.Server(config.rpcUrl);
  const contract = new Contract(config.contractId);

  return {
    async hasEvidenceAccess(playerId, scout) {
      const source = await server.getAccount(scout);
      const transaction = new TransactionBuilder(
        new Account(source.accountId(), source.sequenceNumber()),
        { fee: BASE_FEE, networkPassphrase: config.networkPassphrase }
      )
        .addOperation(
          contract.call(
            "has_evidence_access",
            nativeToScVal(BigInt(playerId), { type: "u64" }),
            new Address(scout).toScVal()
          )
        )
        .setTimeout(30)
        .build();

      const simulation = await server.simulateTransaction(transaction);
      if (rpc.Api.isSimulationError(simulation) || !simulation.result?.retval) {
        throw new Error("Unable to verify evidence access on-chain");
      }

      const result = scValToNative(simulation.result.retval);
      if (typeof result !== "boolean") {
        throw new Error("Unexpected evidence access response from Soroban");
      }
      return result;
    },
  };
}

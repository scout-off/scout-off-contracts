/**
 * Reference implementation: read the verification contract's protocol limits
 * via `get_limits()` instead of hard-coding them in client code.
 *
 * See `docs/CONTRACT_REFERENCE.md` — `get_limits() -> VerificationLimits`.
 */
import { Client as VerificationClient, networks } from "@scoutchain/bindings-verification";

export async function getLimits(network: keyof typeof networks, rpcUrl: string) {
  const client = new VerificationClient({ ...networks[network], rpcUrl });
  const tx = await client.get_limits();
  return tx.result;
}

/** Example pre-validation using the on-chain limits. */
export function isDescriptionValid(description: string, maxDescriptionLen: number): boolean {
  return new TextEncoder().encode(description).length <= maxDescriptionLen;
}

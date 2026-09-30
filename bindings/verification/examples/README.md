# `approveMilestone()` reference implementation

Not generated, not built, not published — these two files exist purely as a
reference for the `scoutchain-backend` team and any frontend integrators
building validator tooling.

## Why this exists

`approve_milestone` is the primary write path for the verification contract:
a registered, active validator signs a transaction attesting that a player has
met a specific performance milestone, and the contract — after confirming the
validator's credentials and the evidence CID's uniqueness — advances the
player's progress level via a cross-contract call to the progress contract.
This directory documents the exact TypeScript call pattern for that flow.

## Files

- `approveMilestone.ts` — builds and submits a real
  `approve_milestone(validator_wallet, player_id, description, evidence_hash)`
  Soroban transaction via `@scoutchain/bindings-verification`'s generated
  `Client`, and returns the 1-indexed milestone index the contract assigned
  (`Result<u32, VerificationError>`) together with the Soroban transaction hash.
  Throws on RPC/simulation failure, on contract-level rejection (e.g.
  `ValidatorInactive`, `DuplicateEvidence`), and on a non-`SUCCESS`
  confirmation status.
- `approveMilestone.integration.test.ts` — calls `approveMilestone()` against
  a live RPC endpoint with a registered active validator and a pre-existing
  player, then re-queries `get_milestone` and `get_milestone_count` on a fresh
  read-only client to confirm the milestone actually landed on-chain. Skips
  automatically unless `INTEGRATION_RPC_URL`, `INTEGRATION_NETWORK`,
  `INTEGRATION_SECRET`, `INTEGRATION_PLAYER_ID`, and
  `INTEGRATION_EVIDENCE_HASH` are all set — see the file header for how to
  run it against testnet.

## Running the integration test

```bash
INTEGRATION_RPC_URL=https://soroban-testnet.stellar.org \
INTEGRATION_NETWORK=testnet \
INTEGRATION_SECRET=S... \
INTEGRATION_PLAYER_ID=1 \
INTEGRATION_EVIDENCE_HASH=QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG \
npx vitest run examples/approveMilestone.integration.test.ts
```

`INTEGRATION_SECRET` must be the secret key of a wallet already registered as
an active validator on that network. `INTEGRATION_EVIDENCE_HASH` must be a
valid IPFS (`Qm…`) or Arweave (`bafy…`) CID, already uploaded, and must be
unique per test run — the contract rejects `DuplicateEvidence`. Append a
timestamp or use a different CID for each run.

## Porting this into `scoutchain-backend`

1. Copy `approveMilestone.ts`'s logic into wherever the backend's validator
   approval endpoint currently stubs or fires a transaction.
2. Swap the `signTransaction` callback for whatever wallet/session signer the
   backend already uses for validator-authorized writes.
3. Upload the evidence file to IPFS/Arweave first; pass the resulting CID as
   `evidenceHash`. The contract validates the CID format on-chain and rejects
   re-used hashes, so each milestone must have a unique, content-addressed CID.
4. Copy the integration test's shape into the backend's existing test harness,
   which should already have a funded validator signer and a player fixture to
   reuse instead of the env-var gating used here.

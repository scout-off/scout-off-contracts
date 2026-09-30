# `updateProfile()` reference implementation

Not generated, not built, not published — these two files exist purely as a
reference for the `scoutchain-backend` team.

## Why this exists

`scoutchain-backend`'s `updateProfile()` currently returns
`stub-update-txid-${playerId.slice(0, 8)}` without submitting anything to the
chain, so player profile updates are silently dropped after IPFS pinning
(tracking issue: "Wire updateProfile() to Soroban update_profile contract
call"). This repo doesn't contain that backend code, but it does own the
contract and the generated bindings the fix depends on, so this directory
documents the exact call pattern to port over.

## Files

- `updateProfile.ts` — builds and submits a real
  `update_profile(player_id, ipfs_hashes)` Soroban transaction via
  `@scoutchain/bindings-registration`'s generated `Client`, and returns the
  transaction hash Soroban RPC assigned to it (`sendTransactionResponse.hash`)
  along with the `metadataUri` that was submitted. Throws on RPC/simulation
  failure, on contract-level rejection (e.g. `PlayerNotFound`), and on a
  non-`SUCCESS` confirmation status.
- `updateProfile.integration.test.ts` — calls `updateProfile()` against a live
  RPC endpoint and a pre-registered player, then re-queries `get_player` on a
  fresh client to confirm the new `metadataUri` is actually readable back from
  the contract. Skips automatically unless `INTEGRATION_RPC_URL`,
  `INTEGRATION_NETWORK`, `INTEGRATION_SECRET`, and `INTEGRATION_PLAYER_ID` are
  all set — see the file header for how to run it against testnet.

## Porting this into `scoutchain-backend`

1. Copy `updateProfile.ts`'s logic into wherever `updateProfile()` currently
   builds the stub ID.
2. Swap the `signTransaction` callback for whatever wallet/session signer the
   backend already uses for player-authorized writes.
3. Copy the integration test's shape into the backend's existing test harness,
   which should already have a funded signer and a registration fixture to
   reuse instead of the env-var gating used here.

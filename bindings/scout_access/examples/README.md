# `subscribeScout()` and `payToContact()` reference implementation

Not generated, not built, not published — these two files exist purely as a
reference for the `scoutchain-backend` team and frontend integrators building
the scout onboarding and player-discovery flow.

## Why this exists

The scout_access contract is the entry point for every paid scout interaction
on ScoutChain. Before a scout can contact players, log trial offers, or access
higher-level profiles, they must hold an active subscription. This directory
documents the exact TypeScript call pattern for that two-step flow:

1. **`subscribe`** — purchase a Basic, Pro, or Elite subscription. The XLM fee
   is transferred from the scout's wallet to the contract atomically.
2. **`pay_to_contact`** — pay a micro-fee to unlock a player's contact details.
   Requires an active Pro or Elite subscription (Basic scouts cannot call this).

Together these two calls represent the minimum working integration surface for
a scout-facing frontend or backend service.

## Files

- `subscribe.ts` — exports `subscribeScout()` and `payToContact()`. Both
  functions build and submit real Soroban transactions via
  `@scoutchain/bindings-scout-access`'s generated `Client` and return the
  Soroban transaction hash. Throws on RPC/simulation failure, on contract-level
  rejection (e.g. `SubscriptionDowngradeNotAllowed`, `ProContactLimitReached`),
  and on a non-`SUCCESS` confirmation status.
- `subscribe.integration.test.ts` — calls `subscribeScout()` followed by
  `payToContact()` against a live RPC endpoint with a funded scout wallet and
  a pre-existing player, then re-queries `get_subscription` and `has_contacted`
  on a fresh read-only client to confirm both operations landed on-chain. Skips
  automatically unless `INTEGRATION_RPC_URL`, `INTEGRATION_NETWORK`,
  `INTEGRATION_SECRET`, `INTEGRATION_TIER`, and `INTEGRATION_PLAYER_ID` are
  all set — see the file header for how to run it against testnet.

## Running the integration test

```bash
# Fund the scout wallet on testnet first if needed:
# curl "https://friendbot.stellar.org?addr=<PUBLIC_KEY>"

INTEGRATION_RPC_URL=https://soroban-testnet.stellar.org \
INTEGRATION_NETWORK=testnet \
INTEGRATION_SECRET=S... \
INTEGRATION_TIER=Pro \
INTEGRATION_PLAYER_ID=1 \
npx vitest run examples/subscribe.integration.test.ts
```

`INTEGRATION_SECRET` must be the secret key of a funded scout wallet. The
wallet needs enough XLM to cover the subscription fee (read from the contract's
`FeeConfig`) plus transaction fees. `INTEGRATION_TIER` must be one of `Basic`,
`Pro`, or `Elite`. The `pay_to_contact` step is automatically skipped when
`INTEGRATION_TIER=Basic`.

## Porting this into `scoutchain-backend`

1. Copy `subscribeScout()` into wherever the backend handles subscription
   purchases. Swap the `signTransaction` callback for whatever wallet/session
   signer the backend uses for scout-authorized writes.
2. Copy `payToContact()` into wherever the backend handles contact-unlock
   requests. Call `client.has_contacted(scout, player_id)` first if you want
   to guard against a redundant fee payment before presenting the signing
   prompt to the user.
3. To read the current fee before displaying it to the user, call
   `client.get_fee_config()` — it returns a `FeeConfig` with
   `basic_sub_stroops`, `pro_sub_stroops`, `elite_sub_stroops`, and
   `contact_fee_stroops` (all in stroops, where 1 XLM = 10,000,000 stroops).
4. The `SubscriptionTier` enum is a Soroban tagged-enum, represented in the
   generated TypeScript bindings as `{ Basic: {} }`, `{ Pro: {} }`, or
   `{ Elite: {} }`. Pass the matching object shape as the `tier` field when
   constructing the `subscribe` call.
5. Copy the integration test's shape into the backend's existing test harness,
   which should already have a funded scout signer and a player fixture to
   reuse instead of the env-var gating used here.

## Trial offer flow (Elite scouts)

The full Elite trial-offer flow (step 3 for Level 3 advancement) is a two-step
sequence that requires the progress contract to be wired:

```
scout  → log_trial_offer(scout, player_id, details_hash)  → returns trial_index
player → confirm_trial_offer(player_wallet, player_id, trial_index)
```

`log_trial_offer` escrows XLM on-chain; `confirm_trial_offer` releases the
escrow and triggers a cross-contract `advance_level` call to the progress
contract, advancing the player to Level 3 (EliteTier). Refer to
`docs/CONTRACT_REFERENCE.md` and `ai.md` for the full parameter types and
error codes for these functions.

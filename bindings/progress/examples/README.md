# `getPlayerHistory()` reference implementation

Not generated, not built, not published — these two files exist purely as a
reference for teams integrating against the ScoutChain progress contract.

## Why this exists

The progress contract's write path (`advance_level`) is cross-contract only: it
is called by the verification contract when a validator approves a milestone,
and by scout_access when a player confirms a trial offer. There is no way to
call it directly from TypeScript. The integration surface that frontend and
backend consumers actually reach for is the read path: "what level is this
player right now, and how did they get there?" This directory documents the
exact call pattern for that read path.

## Files

- `getPlayerHistory.ts` — queries `get_level`, `get_progress_history`,
  `get_history_since`, and `get_progress_history_page` against the progress
  contract via the generated `@scoutchain/bindings-progress` client. Returns
  the player's current `ProgressLevel` string and a typed array of
  `ProgressEntry` records. No wallet or signer is required — all three calls
  are read-only simulations. Throws on RPC/simulation failure or on a
  contract-level error (e.g. `NotInitialized`).
- `getPlayerHistory.integration.test.ts` — calls `getPlayerHistory()` against a
  live RPC endpoint and a player that has at least one on-chain `advance_level`
  entry, then cross-checks the result against a direct `get_history_count` call
  on a fresh client. Skips automatically unless `INTEGRATION_RPC_URL`,
  `INTEGRATION_NETWORK`, and `INTEGRATION_PLAYER_ID` are all set — see the
  file header for how to run it against testnet.

## Running the integration test

```bash
INTEGRATION_RPC_URL=https://soroban-testnet.stellar.org \
INTEGRATION_NETWORK=testnet \
INTEGRATION_PLAYER_ID=1 \
npx vitest run examples/getPlayerHistory.integration.test.ts
```

No funded wallet is needed. `INTEGRATION_PLAYER_ID` must be a numeric
`player_id` belonging to a player who has had `advance_level` called at least
once on-chain (i.e. not `Unverified`).

## Porting this into your integration

1. Copy `getPlayerHistory.ts` into wherever your app queries player progress.
2. The `sinceTimestamp` option is designed for polling indexers: store the
   `updated_at` of the last entry you processed, then pass it on the next poll
   to fetch only new entries instead of re-reading the full history.
3. The `page` option is designed for paginated UI views (e.g. a player's level
   timeline). `limit` is clamped to 50 on-chain; request smaller pages (10–20)
   for responsive UIs.
4. `get_level` never throws `PlayerNotFound` — it returns `Unverified` for
   unknown player IDs. Treat an `Unverified` result from `getPlayerHistory` as
   "player not yet on-chain" rather than as an error.

# Contract Reference

Complete public API reference for all four ScoutChain Soroban smart contracts.
Every `pub fn` in every `#[contractimpl]` block is documented here.

> [!NOTE]
> **Last verified:** 2026-07-26 — manually cross-checked against the contract source and the H2/Table of Contents audit in this documentation-sync PR.

---

All `stellar contract invoke` examples below pass `String` and enum arguments
as JSON values wrapped in shell single quotes, for example `--tier '"Elite"'`.
That keeps the command copy-paste-runnable in a standard `bash`/`zsh` shell.

## Table of Contents

- [registration](#registration)
- [verification](#verification)
- [progress](#progress)
- [scout_access](#scout_access)
- [Shared Types](#shared-types)
  - [`ProgressLevel`](#progresslevel)
  - [`ContractHealth`](#contracthealth)
  - [`PlayerVitals`](#playervitals)
  - [`PlayerProfile`](#playerprofile)
  - [`ScoutProfile`](#scoutprofile)
  - [`Validator`](#validator)
  - [`ValidatorStatus`](#validatorstatus)
  - [`Milestone`](#milestone)
  - [`MilestoneDispute`](#milestonedispute)
  - [`ProgressEntry`](#progressentry)
  - [`SubscriptionTier`](#subscriptiontier)
  - [`Subscription`](#subscription)
  - [`ContactRecord`](#contactrecord)
  - [`FeeConfig`](#feeconfig)
  - [`ProContactPeriod`](#procontactperiod)
  - [`TrialOffer`](#trialoffer)
- [Error Codes](#error-codes)
- [Events](#events)
- [Design Discussion: Check-Ordering Follow-ups](#design-discussion-check-ordering-follow-ups)
- [Cross-Contract Wiring](#cross-contract-wiring)

---

## registration

Handles player and scout on-chain identity: registration, profile updates,
deregistration, and discovery queries.

Timestamp fields returned by this contract (`registered_at` and `updated_at`)
are Unix seconds. See [Timestamp](GLOSSARY.md#timestamp).

### Functions

---

#### `initialize(admin: Address) -> Result<(), ScoutChainError>`

One-time contract setup. Must be called before any other function.

| | |
|---|---|
| **Auth** | `admin` must sign |
| **Errors** | `AlreadyInitialized` if called more than once |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- initialize --admin $ADMIN_ADDRESS
```

---

#### `propose_admin(new_admin: Address) -> Result<(), ScoutChainError>`

Store or replace a pending admin proposal. The current admin retains all
privileges until the proposed address accepts.

| | |
|---|---|
| **Auth** | Current admin must sign |
| **Errors** | `NotInitialized` |
| **Emits** | `admin_transfer_proposed` with `(old_admin, new_admin)` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- propose_admin --new_admin $NEW_ADMIN_ADDRESS
```

---

#### `accept_admin() -> Result<(), ScoutChainError>`

Finalize the pending transfer. The stored pending admin must sign, proving
control of the address. Acceptance updates the admin and clears the proposal.

| | |
|---|---|
| **Auth** | Pending admin must sign |
| **Errors** | `NotInitialized` · `PendingAdminNotSet` |
| **Emits** | `admin_transferred` with `(old_admin, new_admin)` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID -- accept_admin
```

---

#### `transfer_admin(new_admin: Address) -> Result<(), ScoutChainError>`

Deprecated compatibility alias for `propose_admin`. It does not immediately
change the admin; the proposed address must still call `accept_admin`.

---

#### `register_player(wallet: Address, vitals: PlayerVitals, ipfs_hashes: Vec<String>) -> Result<u64, ScoutChainError>`

Create a new on-chain player profile at Level 0 (Unverified).
Returns the assigned `player_id`.

| | |
|---|---|
| **Auth** | `wallet` must sign |
| **Errors** | `AlreadyRegistered` · `InvalidInput` (field too long or bad hash count) · `NotInitialized` · `ContractPaused` · `Overflow` |

Constraints:
- `position` and `nationality` max 64 bytes each; `region` max 100 bytes
- `ipfs_hashes` must contain 1–10 entries
- Player vitals (`position`, `region`, `nationality`, `age`) are write-once at registration time and immutable post-registration. Length limits are strictly enforced during `register_player` and cannot be bypassed via post-registration mutation.

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- register_player \
  --wallet $PLAYER_ADDRESS \
  --vitals '{"age":20,"position":"Forward","region":"West Africa","nationality":"Ghana"}' \
  --ipfs_hashes '["QmHighlightCID"]'
```

---

#### `update_profile(player_id: u64, ipfs_hashes: Vec<String>) -> Result<(), ScoutChainError>`

Replace a player's IPFS content hashes (highlight reels, photos). Note that `update_profile` accepts only `ipfs_hashes` and does not take or modify `PlayerVitals` fields. Because player vitals are write-once at registration time and immutable post-registration, length validation runs exclusively during `register_player` and no post-registration update path exists to set or modify vitals.

| | |
|---|---|
| **Auth** | Player's wallet must sign |
| **Errors** | `PlayerNotFound` · `InvalidInput` (empty or >10 hashes) · `ContractPaused` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- update_profile \
  --player_id 1 \
  --ipfs_hashes '["QmNewCID1","QmNewCID2"]'
```

---

#### `deregister_player(player_id: u64) -> Result<(), ScoutChainError>`

Remove a player profile and all associated wallet index entries.
Implements the GDPR right-to-erasure. The `player_id` is permanently freed.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `PlayerNotFound` · `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- deregister_player --player_id 1
```

---

#### `deactivate_player(player_id: u64) -> Result<(), ScoutChainError>`

Hide a player from `filter_players` results without erasing their profile
(soft-delete). Sets the `PlayerDeactivated` flag; the player's data and
`player_id` remain intact and can be restored with `reactivate_player`.
Emits a `player_deactivated` event on success.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `PlayerNotFound` · `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- deactivate_player --player_id 1
```

---

#### `reactivate_player(player_id: u64) -> Result<(), ScoutChainError>`

Reverse a prior `deactivate_player` call. Clears the `PlayerDeactivated`
flag, making the player visible in `filter_players` results again.
Emits a `player_reactivated` event on success.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `PlayerNotFound` · `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- reactivate_player --player_id 1
```

---

#### `register_scout(wallet: Address, region: String) -> Result<u64, ScoutChainError>`

Create a new scout profile. Returns the assigned `scout_id`.
Scouts start as unverified (`verified: false`); call `verify_scout` to promote.

| | |
|---|---|
| **Auth** | `wallet` must sign |
| **Errors** | `AlreadyRegistered` · `InvalidInput` (region >128 bytes) · `NotInitialized` · `ContractPaused` · `Overflow` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- register_scout \
  --wallet $SCOUT_ADDRESS \
  --region '"West Africa"'
```

---

#### `verify_scout(scout_id: u64) -> Result<(), ScoutChainError>`

Mark a scout as verified. Verified scouts gain trust-signal visibility on the
discovery dashboard.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ScoutNotFound` · `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- verify_scout --scout_id 1
```

---

#### `set_progress_contract(addr: Address) -> Result<(), ScoutChainError>`

Store the progress contract address so `set_player_level` may only be called
by that contract. Must be called after both contracts are deployed (admin only).
Freely re-settable — no guard. Bumps the link's re-wiring epoch and emits
`wiring_updated` (`link = "progress_contract"`) on every call (issue #1041).

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- set_progress_contract --addr $PROGRESS_CONTRACT_ID
```

---

#### `get_progress_contract() -> Option<Address>`

Returns the configured progress contract address, or `None` when the link has
not yet been configured. Read-only and requires no auth.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `get_wiring_state() -> RegistrationWiringState`

Returns a snapshot of the single peer-address pointer this contract holds
(`progress_contract`), paired with its re-wiring epoch (`0` iff unset).
`RegistrationWiringState::is_fully_wired()` returns `true` iff the address is
set. Read-only, no auth required — see
[`docs/WIRING_REGISTRY_DESIGN.md`](WIRING_REGISTRY_DESIGN.md).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_wiring_state
```

---

#### `set_player_level(player_id: u64, level: ProgressLevel) -> Result<(), ScoutChainError>`

Update a player's stored `ProgressLevel`. Only callable by the registered
progress contract address via cross-contract invocation.

| | |
|---|---|
| **Auth** | Registered progress contract must sign |
| **Errors** | `Unauthorized` (progress contract not configured or wrong caller) · `PlayerNotFound` |

_Not intended for direct invocation. Called atomically by `progress.advance_level`._

> **Idempotent — safe to retry (Issue #811 follow-up).** Unlike
> `progress.advance_level`, this function is a *keyed absolute write*, not a
> relative step: it sets the player's level to the supplied `level` rather than
> deriving a next value. Replaying it with the same arguments converges on the
> same state.
>
> The index maintenance is idempotent by construction: before adding the player
> to the target bucket it removes them from **all four** level buckets, so a
> replay cannot produce duplicate entries in `PlayersByLevel` or
> `PlayersByLevelRegion` even though the underlying `level_index_add` /
> `composite_index_add` helpers are unguarded `push_back`s.
>
> The only field that differs across replays is `updated_at` on the stored
> profile, plus a repeated `player_level_synced` event and an extended
> `PlayerLevel` TTL — none of which are state-corrupting. A replay against a
> deleted player fails cleanly with `PlayerNotFound`.

---

#### `get_player(player_id: u64) -> Result<PlayerProfile, ScoutChainError>`

Retrieve the full player profile including wallet, vitals, IPFS hashes, and
current progress level.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `PlayerNotFound` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_player --player_id 1
```

---

#### `get_player_by_wallet(wallet: Address) -> Result<PlayerProfile, ScoutChainError>`

Look up a player profile by their Stellar wallet address. Useful when the
`player_id` is unknown.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `PlayerNotFound` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_player_by_wallet --wallet $PLAYER_ADDRESS
```

---

#### `get_scout(scout_id: u64) -> Result<ScoutProfile, ScoutChainError>`

Retrieve a scout profile by ID.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `ScoutNotFound` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_scout --scout_id 1
```

---

#### `get_scout_by_wallet(wallet: Address) -> Result<ScoutProfile, ScoutChainError>`

Retrieve a scout profile by wallet address, resolving the wallet to a
`scout_id` via the `DataKey::ScoutByWallet` index and delegating to `get_scout`.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `ScoutNotFound` |
| **Cross-contract calls** | Called *by* the `scout_access` contract's `subscribe()` — see [`docs/SYBIL_MITIGATION_DESIGN.md`](SYBIL_MITIGATION_DESIGN.md#cross-contract-integration). Before allowing a Pro-tier subscription, `subscribe()` calls this function to fetch `ScoutProfile.verification.verified` and rejects with `ScoutAccessError::ScoutNotVerified` (scout_access error code 27) if the scout isn't found or isn't verified. This read is not atomic with the subscription write — a scout's verification status is read at call time, so a verification revoked in the same ledger as a competing `subscribe()` call is not guaranteed to be observed. |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_scout_by_wallet --wallet $SCOUT_ADDRESS
```

---

#### `get_scout_verification(scout_id: u64) -> Result<ScoutVerificationRecord, ScoutChainError>`

Retrieve just the structured verification record (`verified`, `verified_by`,
`verified_at`, `evidence_ref`, `method`) for a scout by ID, without the rest
of the `ScoutProfile`.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `ScoutNotFound` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_scout_verification --scout_id 1
```

---

#### `get_player_count() -> u64`

Return the total number of registered players. Returns `0` before the contract
is initialized.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID -- get_player_count
```

---

#### `get_player_summary(player_id: u64) -> Result<PlayerSummary, ScoutChainError>`

Return a lightweight player summary (vitals + level, no IPFS hashes or wallet)
for efficient list rendering on the scout discovery dashboard.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `PlayerNotFound` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_player_summary --player_id 1
```

---

#### `get_players(ids: Vec<u64>) -> Result<Vec<PlayerSummary>, ScoutChainError>`

Batch-fetch player summaries for a list of IDs. Unknown IDs are skipped.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `NotInitialized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_players --ids '[1,2,3]'
```

---

#### `get_scout_count() -> u64`

Return the total number of registered scouts. Returns `0` before the contract
is initialized.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID -- get_scout_count
```

---

#### `filter_players(region: String, position: String, min_level: ProgressLevel, offset: u32, limit: u32) -> Result<FilterResult, ScoutChainError>`

Scout discovery query. Returns up to 50 player profiles matching the given
region, position, and minimum progress level.

Uses the composite `PlayersByLevelRegion(level, region)` index as the entry
point so only players that already satisfy the level+region criteria are loaded.
Gas cost is proportional to the number of matching players, not the total player
count. The index is maintained automatically on `register_player`,
`set_player_level`, and `deregister_player`.

Pagination:
- `offset` = 0 starts from the beginning.
- Pass the previously returned `FilterResult.next_cursor` value as `offset` to
  fetch the next page.
- `next_cursor` = 0 in the response means no further results.
- Both `offset` and `next_cursor` are *counts* of eligible (non-deactivated,
  filter-matching) entries, not player IDs.

> **Past bug (#1017):** `next_cursor` used to be set to the raw `player_id` of
> the last entry on the page while `offset` was always compared as a count of
> eligible entries — different units that only coincided by accident when
> player IDs were contiguous with no filter gaps. Paginating past a
> non-matching player (e.g. a different position) between pages would skip
> one eligible entry per such gap. Fixed by making `next_cursor` a count in
> the same unit as `offset`, matching the contract documented above; both
> code paths (the region-filtered fast path and the full-scan slow path) and
> their doc comments were updated together, and a regression test
> (`test_filter_players_pagination_cursor_no_gaps`) walks a filtered page
> boundary to assert no entries are skipped or duplicated.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `NotInitialized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- filter_players \
  --region '"West Africa"' \
  --position '"Forward"' \
  --min_level '"Unverified"' \
  --offset 0 \
  --limit 50
```

---

#### `pause_contract() -> Result<(), ScoutChainError>`

Halt all state-changing operations (circuit breaker). Read-only queries remain
available.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID -- pause_contract
```

---

#### `unpause_contract() -> Result<(), ScoutChainError>`

Resume normal operations after a pause.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID -- unpause_contract
```

---

#### `health() -> ContractHealth`

Return the contract's initialization and pause status.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID -- health
```

---

#### `get_player_summary(player_id: u64) -> Result<PlayerSummary, ScoutChainError>`

Return a lightweight player view without IPFS hashes or wallet address.
Useful for scout discovery lists where the full profile is not needed.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `PlayerNotFound` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_player_summary --player_id 1
```

---

#### `get_players(ids: Vec<u64>) -> Result<Vec<PlayerSummary>, ScoutChainError>`

Batch-fetch lightweight player summaries for up to 20 IDs in a single call.
Missing IDs are silently skipped (partial hits are returned without error).
For cost rationale behind batch-size caps, see the batch-operation entries in [`ci/cpu-cost-budget.md`](../ci/cpu-cost-budget.md), including `scout_access.batch_contact_players`.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `InvalidInput` (more than 20 IDs provided) |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_players --ids '[1,2,3]'
```

---

#### `get_scouts(ids: Vec<u64>) -> Result<Vec<ScoutProfile>, ScoutChainError>`

Batch-fetch full scout profiles for up to 20 IDs in a single call. Mirrors
`get_players` semantics exactly: missing IDs are silently skipped with partial
hits returned successfully, and the same 20-ID cap applies.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `InvalidInput` (more than 20 IDs provided) |

**Examples**:
```bash
# Fetch three scouts by ID
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_scouts --ids '[1,2,3]'

# Mixed batch with one nonexistent ID — returns two profiles only
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_scouts --ids '[1,999,2]'
```

---

#### `version() -> String`

Return the deployed contract version string (from `Cargo.toml` at build time).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID -- version
```

---

#### `redeem_migration_player(wallet: Address, vitals: PlayerVitals, ipfs_hashes: Vec<String>, level: ProgressLevel, player_id: u64, registered_at: u64, updated_at: u64, authorization: MigrationAuthorization) -> Result<u64, ScoutChainError>`

Redeem an off-chain signed migration authorization to recreate a player profile
on a freshly deployed contract. A relayer with no player private key can call
this function; the player's ed25519 signature over the canonical authorization
message serves as proof of consent.

The signed message covers: `wallet || role(Player=0) || profile_data_hash || new_contract_hint || nonce || expires_at`. The same nonce cannot be reused (replay protection).

| | |
|---|---|
| **Auth** | None (signature-based authorization) |
| **Errors** | `NotInitialized` · `ContractPaused` · `InvalidInput` (bad signature, expired, wrong role, mismatched hash, or replayed nonce) · `PlayerNotFound` (if player already exists) · `AlreadyRegistered` |

```bash
stellar contract invoke --id $NEW_REGISTRATION_CONTRACT_ID \
  -- redeem_migration_player \
  --wallet $PLAYER_ADDRESS \
  --vitals '{"age":20,"position":"Forward","region":"West Africa","nationality":"Ghana"}' \
  --ipfs_hashes '["QmHighlightCID"]' \
  --level Unverified \
  --player_id 1 \
  --registered_at 1700000000 \
  --updated_at 1700000000 \
  --authorization '{"wallet":"$PLAYER_ADDRESS","role":"Player","profile_data_hash":"<sha256>","new_contract_hint":"$NEW_CONTRACT_ID","nonce":1,"expires_at":0,"signature":"<base64>"}'
```

---

#### `redeem_migration_scout(wallet: Address, region: String, scout_id: u64, registered_at: u64, verified: bool, authorization: MigrationAuthorization) -> Result<u64, ScoutChainError>`

Redeem an off-chain signed migration authorization to recreate a scout profile
on a freshly deployed contract. A relayer with no scout private key can call
this function; the scout's ed25519 signature over the canonical authorization
message serves as proof of consent.

The signed message covers: `wallet || role(Scout=1) || region_hash || new_contract_hint || nonce || expires_at`.

| | |
|---|---|
| **Auth** | None (signature-based authorization) |
| **Errors** | `NotInitialized` · `ContractPaused` · `InvalidInput` (bad signature, expired, wrong role, mismatched hash, or replayed nonce) · `ScoutNotFound` (if scout already exists) · `AlreadyRegistered` |

```bash
stellar contract invoke --id $NEW_REGISTRATION_CONTRACT_ID \
  -- redeem_migration_scout \
  --wallet $SCOUT_ADDRESS \
  --region '"West Africa"' \
  --scout_id 1 \
  --registered_at 1700000000 \
  --verified false \
  --authorization '{"wallet":"$SCOUT_ADDRESS","role":"Scout","profile_data_hash":"<sha256>","new_contract_hint":"$NEW_CONTRACT_ID","nonce":1,"expires_at":0,"signature":"<base64>"}'
```

---

### Dual-Role Wallet Policy

A single wallet may register as both a player and a scout. Cross-role
registration is permitted; duplicate prevention is enforced per role only.

### ScoutChainError Codes

| Code | Error | Description |
|------|-------|-------------|
| 1 | `AlreadyInitialized` | Contract has already been initialized |
| 2 | `NotInitialized` | Contract has not been initialized yet |
| 3 | `PlayerNotFound` | Player ID does not exist |
| 4 | `ValidatorNotAuthorized` | Caller is not a registered and active validator |
| 5 | `InvalidProgressTransition` | Requested level transition is not allowed |
| 6 | `ScoutNotSubscribed` | Scout does not have an active subscription |
| 7 | `InsufficientFee` | Payment amount is below the required fee |
| 8 | `AlreadyRegistered` | Wallet already has a registered profile |
| 9 | `ContractPaused` | Contract is paused by the emergency circuit breaker |
| 10 | `Unauthorized` | Caller is not authorized for the requested operation |
| 11 | `Overflow` | Arithmetic overflow in fee calculation |
| 12 | `ScoutNotFound` | Scout ID does not exist |
| 13 | `InvalidInput` | One or more input parameters are invalid |
| 14 | `ValidatorCapReached` | Maximum number of registered validators has been reached |
| 15 | `PlayerCapReached` | Maximum number of registered players has been reached |
| 16 | `RegistrationCooldown` | Registration attempted before the cooldown period has elapsed |
| 17 | `PlayerRecordEvicted` | Player record was evicted from contract storage |
| 18 | `ScoutRecordEvicted` | Scout record was evicted from contract storage |

---

## verification

Manages the trusted validator registry and milestone approvals. Cross-calls
`progress.advance_level` atomically when a milestone is approved.

Timestamp fields returned by this contract (`registered_at`, `approved_at`, and
`disputed_at`) are Unix seconds. See [Timestamp](GLOSSARY.md#timestamp).

### Functions

---

#### `initialize(admin: Address) -> Result<(), VerificationError>`

One-time contract setup.

| | |
|---|---|
| **Auth** | `admin` must sign |
| **Errors** | `AlreadyInitialized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- initialize --admin $ADMIN_ADDRESS
```

---

#### `propose_admin(new_admin: Address) -> Result<(), VerificationError>`

Store or replace a pending admin proposal. The current admin retains all
privileges until the proposed address accepts.

| | |
|---|---|
| **Auth** | Current admin must sign |
| **Errors** | `NotInitialized` |
| **Emits** | `admin_transfer_proposed` with `(old_admin, new_admin)` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- propose_admin --new_admin $NEW_ADMIN_ADDRESS
```

---

#### `accept_admin() -> Result<(), VerificationError>`

Finalize the pending transfer. The stored pending admin must sign, proving
control of the address. Acceptance updates the admin and clears the proposal.

| | |
|---|---|
| **Auth** | Pending admin must sign |
| **Errors** | `NotInitialized` · `PendingAdminNotSet` |
| **Emits** | `admin_transferred` with `(old_admin, new_admin)` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- accept_admin
```

---

#### `transfer_admin(new_admin: Address) -> Result<(), VerificationError>`

Deprecated compatibility alias for `propose_admin`. It does not immediately
change the admin; the proposed address must still call `accept_admin`.

---

#### `set_progress_contract(progress_contract: Address) -> Result<(), VerificationError>`

Wire the progress contract address so `approve_milestone` can call
`advance_level` cross-contract. Must be called once after deployment.
Returns `AlreadyConfigured` on subsequent calls — use
`update_progress_contract` for intentional re-wiring. This first-call-only
guard is deliberately preserved (issue #1041) for backward compatibility with
already-deployed contracts, unlike every other `set_*_contract` setter across
all four contracts, which is freely re-settable. Bumps the link's re-wiring
epoch and emits `wiring_updated` (`link = "progress_contract"`) in addition
to `progress_contract_updated`.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `AlreadyConfigured` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- set_progress_contract --progress_contract $PROGRESS_CONTRACT_ID
```

---

#### `update_progress_contract(progress_contract: Address) -> Result<(), VerificationError>`

Re-wire the progress contract address after the initial `set_progress_contract`
call. Use when redeploying or rotating the progress contract. Also bumps the
link's re-wiring epoch and emits `wiring_updated`, same as `set_progress_contract`.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- update_progress_contract --progress_contract $NEW_PROGRESS_CONTRACT_ID
```

---

#### `get_progress_contract() -> Option<Address>`

Returns the configured progress contract address, or `None` when the link has
not yet been configured. Read-only and requires no auth.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `set_registration_contract(addr: Address) -> Result<(), VerificationError>`

Store the registration contract address so `dispute_milestone` can verify
wallet-to-player-id ownership via cross-contract call. Must be called once
after deployment. Returns `AlreadyConfigured` on subsequent calls — use
`update_registration_contract` for intentional re-wiring. Same
deliberately-preserved first-call-only guard as `set_progress_contract`
above (issue #1041). Bumps the link's re-wiring epoch and emits
`wiring_updated` (`link = "registration_contract"`).

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `AlreadyConfigured` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- set_registration_contract --addr $REGISTRATION_CONTRACT_ID
```

---

#### `update_registration_contract(addr: Address) -> Result<(), VerificationError>`

Re-wire the registration contract address after the initial
`set_registration_contract` call. Use when redeploying or rotating the
registration contract. Also bumps the link's re-wiring epoch and emits
`wiring_updated`, same as `set_registration_contract`.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- update_registration_contract --addr $NEW_REGISTRATION_CONTRACT_ID
```

---

#### `get_wiring_state() -> VerificationWiringState`

Returns a snapshot of both peer-address pointers this contract holds
(`progress_contract`, `registration_contract`), each as a
`WiringLink { address: Option<Address>, epoch: u32 }`
(`scoutchain_shared_types::WiringLink`). `VerificationWiringState::is_fully_wired()`
returns `true` iff both links are configured. Read-only, no auth required —
see [`docs/WIRING_REGISTRY_DESIGN.md`](WIRING_REGISTRY_DESIGN.md).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_wiring_state
```

---

#### `register_validator(wallet: Address, credentials: String, specializations: Vec<String>) -> Result<(), VerificationError>`

Onboard a new trusted validator (coach, academy director, certified trainer).
`credentials` is a human-readable label (max 256 bytes, e.g. `"UEFA B License"`).
`specializations` is an optional list of category tags (max 10 tags, each tag
max 64 bytes, e.g. `["physical-stats", "identity-kyc"]`). Pass an empty `Vec`
for a general-purpose validator that can approve any untagged (general-category)
milestone. When a tagged milestone category is provided to `approve_milestone`,
only validators whose `specializations` list contains that category can approve it.

The contract enforces a cap of **100 simultaneously registered validators**. This limit exists because all validator addresses are stored in a single persistent entry; exceeding Soroban's 64 KB per-entry limit would cause the entry to become unreadable. Raising the cap requires a contract upgrade.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorAlreadyRegistered` · `InvalidInput` (credentials >256 bytes, or >10 specializations, or empty/oversized tag) · `ValidatorCapReached` (100-validator limit reached) · `NotInitialized` · `ContractPaused` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- register_validator \
  --wallet $VALIDATOR_ADDRESS \
  --credentials '"UEFA B License"' \
  --specializations '["physical-stats"]'
```

---

#### `revoke_validator(wallet: Address, severity: RevocationSeverity, reason: Option<String>) -> Result<(), VerificationError>`

Deactivate a validator. Revoked validators cannot approve milestones.

`severity` must be one of:

- `RevocationSeverity::Routine` — deactivates the validator only; no milestone flags are changed.
- `RevocationSeverity::ForCause` — deactivates the validator **and** starts a bounded cascade sweep that flags every milestone the validator previously approved as `MilestonePendingReReview` (see below). If the validator has more than 50 prior approvals, `continue_revocation_cascade` must be called to finish the sweep.

`reason` is optional and capped at 128 bytes. A `RevocationRecord` (severity, reason, timestamp, admin) is persisted under `DataKey::RevocationRecord(wallet)`.

**Breaking change (v1.0.0):** The old `reason: Option<String>` signature is replaced. The old string-equality-to-`"Routine"` severity inference is removed. All call sites must supply an explicit `severity`.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorNotFound` · `ReasonTooLong` (reason > 128 bytes) · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- revoke_validator \
  --wallet $VALIDATOR_ADDRESS \
  --severity '{"ForCause": null}' \
  --reason '"Fabricated credentials"'
```

---

#### `continue_revocation_cascade(wallet: Address) -> Result<(), VerificationError>`

Resume an in-progress for-cause revocation cascade sweep. Call repeatedly (admin only) until the `revocation_cascade_complete` event is emitted. If no cascade is in progress (all milestones already flagged), this is a no-op.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorNotFound` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- continue_revocation_cascade \
  --wallet $VALIDATOR_ADDRESS
```

---

#### `is_milestone_flagged(player_id: u64, milestone_index: u32) -> bool`

Returns `true` if the milestone is currently flagged as pending re-review due to a for-cause validator revocation cascade. Returns `false` if never flagged or already cleared.

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- is_milestone_flagged \
  --player_id 42 \
  --milestone_index 1
```

---

#### `rereview_milestone(reviewer: Address, player_id: u64, milestone_index: u32) -> Result<(), VerificationError>`

Clear a `MilestonePendingReReview` flag after independently confirming the underlying achievement. The `reviewer` must be a currently-active validator (not necessarily the original approver). Emits `milestone_flag_cleared`.

| | |
|---|---|
| **Auth** | `reviewer` must sign |
| **Errors** | `MilestoneNotFound` · `NotEligibleToReReview` (reviewer not active) · `MilestoneNotFlagged` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- rereview_milestone \
  --reviewer $REVIEWER_ADDRESS \
  --player_id 42 \
  --milestone_index 1
```

---

#### `get_revocation_record(wallet: Address) -> Option<RevocationRecord>`

Return the stored `RevocationRecord` for a revoked validator, if any. Returns `None` if the validator has never been revoked via the severity-aware path.

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_revocation_record \
  --wallet $VALIDATOR_ADDRESS
```

---

#### `batch_revoke_validators(wallets: Vec<Address>, severity: RevocationSeverity, reason: Option<String>) -> Result<(), VerificationError>`

Revoke multiple validators in a single atomic transaction. Applies the same
revoke logic as `revoke_validator` to each wallet in `wallets`, emitting one
`validator_revoked` event per revocation (and `validator_revoked_for_cause` for ForCause). If any wallet is not registered, the entire batch fails and no revocations are applied.

For `ForCause`, each validator's cascade sweep is started inline. Use `continue_revocation_cascade` for any validator whose prior approval history exceeds the 50-entry per-call limit.

**Breaking change (v1.0.0):** `reason: Option<String>` is replaced by `severity: RevocationSeverity` + `reason: Option<String>`.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorNotFound` · `ReasonTooLong` (reason > 128 bytes) · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- batch_revoke_validators \
  --wallets '["'$VALIDATOR_ADDRESS_1'","'$VALIDATOR_ADDRESS_2'"]' \
  --severity '{"Routine": null}' \
  --reason '"Season review"'
```

---

#### `restore_validator(wallet: Address) -> Result<(), VerificationError>`

Re-activate a previously revoked validator. The validator's credentials and
milestone history are preserved — only the `active` flag is flipped back to
`true`, so they can immediately approve milestones again.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorNotFound` · `Overflow` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- restore_validator --wallet $VALIDATOR_ADDRESS
```

---

#### `set_validator_specializations(wallet: Address, specializations: Vec<String>) -> Result<(), VerificationError>`

Update the specialization tags for an existing validator. Replaces the
validator's current `specializations` list with the supplied one. Pass an
empty `Vec` to make the validator general-purpose (untagged, can approve any
untagged milestone). Max 10 tags, each max 64 bytes.

This is additive/non-breaking: validators with no specializations remain
fully functional for untagged milestones; specialization checks only engage
when `approve_milestone` is called with a non-`None` `milestone_category`.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorNotFound` · `InvalidInput` (>10 tags or empty/oversized tag) · `NotInitialized` · `ContractPaused` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- set_validator_specializations \
  --wallet $VALIDATOR_ADDRESS \
  --specializations '["physical-stats","match-performance"]'
```

---

#### `transfer_validator(old_wallet: Address, new_wallet: Address) -> Result<(), VerificationError>`

Migrate a validator's identity to a new wallet address. Copies the
`Validator` record (credentials, registration timestamp, active flag) and the
per-validator milestone count to `new_wallet`, then removes `old_wallet`'s
storage entries and swaps it for `new_wallet` in the validator registry.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorNotFound` (old_wallet not registered) · `ValidatorAlreadyRegistered` (new_wallet already registered, including the same-address case where `old_wallet == new_wallet`) · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- transfer_validator \
  --old_wallet $OLD_VALIDATOR_ADDRESS \
  --new_wallet $NEW_VALIDATOR_ADDRESS
```

---

#### `register_validator_with_attestation(wallet: Address, attestation: CredentialAttestation) -> Result<(), VerificationError>`

Register a validator with a cryptographically verified credential attestation. The
`attestation` must contain a valid ed25519 signature produced by a trusted issuer
over the structured claim `(validator_wallet || credential_type || expires_at)`.
The issuer's public key is derived from their registered wallet address.

This path requires the issuer to be pre-registered in the issuer registry (via
`register_issuer`). If the issuer is not yet onboarded, use the legacy
`register_validator` admin-vouched path instead.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NotInitialized` · `ContractPaused` · `InvalidInput` · `CredentialExpired` · `UntrustedIssuer` · `InvalidAttestation` · `ValidatorAlreadyRegistered` · `ValidatorCapReached` · `Overflow` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- register_validator_with_attestation \
  --wallet $VALIDATOR_ADDRESS \
  --attestation '{"validator_wallet":"$ISSUER_ADDRESS","credential_type":"UEFA B License","expires_at":0,"signature":"<base64>"}'
```

---

#### `register_issuer(wallet: Address, name: String) -> Result<(), VerificationError>`

Register a trusted credential issuer (e.g. a football federation) authorized to
sign validator attestation claims. The issuer's wallet address serves as their
ed25519 public key identifier.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NotInitialized` · `ContractPaused` · `InvalidInput` · `IssuerCapReached` (20-issuer limit) · `IssuerAlreadyRegistered` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- register_issuer \
  --wallet $ISSUER_ADDRESS \
  --name '"Football Federation"'
```

---

#### `revoke_issuer(wallet: Address) -> Result<(), VerificationError>`

Deactivate an issuer. Revoked issuers cannot sign new attestations; existing
attestations remain valid.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `IssuerNotFound` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- revoke_issuer --wallet $ISSUER_ADDRESS
```

---

#### `get_issuer(wallet: Address) -> Option<Issuer>`

Retrieve an issuer record by wallet address.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `list_issuers() -> Vec<Address>`

List all registered issuer wallets.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `get_issuer_count() -> u32`

Return the total number of registered issuers.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `approve_milestone(validator_wallet: Address, player_id: u64, description: String, evidence_hash: String) -> Result<u32, VerificationError>`
#### `approve_milestone(validator_wallet: Address, player_id: u64, description: String, evidence_hash: String, milestone_category: Option<String>) -> Result<u32, VerificationError>`

Record a verified milestone for a player. Caller must be a registered, active
validator. Evidence hash must be a valid IPFS (`Qm…`) or Arweave (`bafy…`) CID
of 2–128 bytes.

`milestone_category` is an optional specialization tag (max 64 bytes, e.g.
`"physical-stats"` or `"identity-kyc"`). When supplied, the validator's
`specializations` list must contain this category — if not, the call is
rejected with `SpecializationMismatch`. When omitted (`None`), the
specialization check is skipped entirely and any active validator can approve,
preserving the existing untagged behaviour for backwards compatibility.

**Milestone Examples:**

| Description | Category | Required validator specialization |
|---|---|---|
| "Scored 5 goals in Local Cup" | `None` (untagged) | Any active validator |
| "Top speed clocked at 32 km/h" | `"physical-stats"` | Validator with `"physical-stats"` |
| "Academy confirms active membership" | `"identity-kyc"` | Validator with `"identity-kyc"` |

After storing the milestone this function cross-calls `progress.advance_level`
atomically so both state changes occur in the same Stellar transaction. Returns
the milestone index.

**Single-validator trust model — closed once k-of-n mode is configured.**
`approve_milestone` commits on the strength of exactly one validator's
signature. As soon as an operator calls `set_milestone_threshold(n)` with
`n >= 2`, `approve_milestone` starts rejecting every call with
`ThresholdModeRequiresAttestation` — there is no single-signature bypass once
k-of-n mode is active. The default threshold is `1`, which reproduces
`approve_milestone`'s historical behaviour unchanged; this is a deliberate,
well-gated degenerate case kept so every existing integrator (registration,
scout_access, chaos-tests, and this contract's own pre-existing callers) keeps
working without a coordinated migration, not a silent escape hatch. Operators
who actually want to close the single-compromised-validator gap described in
the k-of-n threshold attestation design below must call
`set_milestone_threshold(n)` with `n >= 2`. See `attest_milestone`.

| | |
|---|---|
| **Auth** | `validator_wallet` must sign |
| **Errors** | `ContractPaused` · `ThresholdModeRequiresAttestation` (k-of-n mode is configured — use `attest_milestone` instead) · `ValidatorNotFound` · `ValidatorInactive` · `InvalidInput` (bad evidence hash or category tag >64 bytes) · `DuplicateEvidence` (evidence hash already used) · `MilestoneLimitExceeded` (5 milestones/player/validator cap) · `SpecializationMismatch` (category provided but validator not tagged for it) · `Overflow` · `ProgressCallFailed` |

```bash
# Untagged milestone (any active validator)
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- approve_milestone \
  --validator_wallet $VALIDATOR_ADDRESS \
  --player_id 1 \
  --description '"Scored 5 goals in Local Cup"' \
  --evidence_hash '"QmEvidence123"' \
  --milestone_category null

# Tagged milestone (only validators specialised in physical-stats)
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- approve_milestone \
  --validator_wallet $TRAINER_ADDRESS \
  --player_id 1 \
  --description '"Top speed clocked at 32 km/h"' \
  --evidence_hash '"QmEvidence456"' \
  --milestone_category '"physical-stats"'
```

---

### k-of-n threshold milestone attestation

`approve_milestone`'s single-signature trust model means one compromised,
colluding, or simply mistaken validator can unilaterally mint a milestone and
trigger an irreversible-by-default player-tier advance. `attest_milestone`
replaces that with an on-chain **accumulation** pattern: since this platform's
validators are geographically distributed coaches/academy directors (see
[README "Validator Network"](../README.md)) who cannot practically co-sign a
single Soroban transaction together, each validator submits their own
attestation independently — potentially hours or days apart — and the
contract tallies distinct votes in bounded storage until a configurable
`threshold` is reached, only then committing the milestone and cross-calling
`progress.advance_level`.

**Claim identity: `(player_id, evidence_hash)`, not `description`.** Two
validators attesting with independently-worded descriptions for the same
evidence still corroborate the same claim. The alternative — requiring an
exact `description` match — was rejected: wording variance alone would
fracture legitimate consensus (a validator who paraphrases "hat-trick in cup
final" as "3 goals, regional final" would silently open a second, disjoint
claim instead of corroborating the first), which is a subtler and
easier-to-trigger griefing vector than trusting the immutable evidence
artifact the CID already represents. The description recorded on the
committed `Milestone` is locked in by the first vote in each round and is
never overwritten by later voters, so the threshold-reaching validator cannot
rewrite the claim's narrative at the last moment either.

**Bounded, O(1)-per-vote storage.** Each claim is one fixed-size
`PendingMilestoneClaim` record (a vote counter, not a growing list of voter
addresses) plus one fixed-size existence marker per `(claim, validator)` pair
used for duplicate-vote rejection. This deliberately avoids the
monolithic-Vec-rewrite anti-pattern present elsewhere in this codebase — see
`cost_attest_milestone_threshold_reach_does_not_scale_with_vote_count` in
`contracts/verification/tests/threshold_milestone_attestation.rs`, which
measures the CPU-instruction cost (via `env.cost_estimate().budget()`) of the
threshold-reaching call at `threshold = 5` and `threshold = 20` and asserts
the growth stays well under what an O(n) voter-list rewrite would produce.

**Duplicate votes** from the same validator on the same claim/round are
rejected with `DuplicateAttestation` — a distinct, differentiated result from
a first-time `AttestationStatus::Pending`/`Committed`, not a silent no-op.

**Revoke-during-pending-vote policy: retroactive invalidation.** If
`revoke_validator` (or `batch_revoke_validators`) is called against a
validator with a still-open vote on a sub-threshold claim, that vote is
stripped from the claim's tally immediately, in the same transaction — the
claim then needs a fresh vote from a different active validator to make up
the difference. This is enforced by a bounded, capped
(`MAX_PENDING_VOTES_PER_VALIDATOR` = 25) per-validator index of open votes
that revocation walks and reverses, not merely documented behaviour. The
alternative (grandfathering a revoked validator's vote) would mean a
validator revoked specifically *because* they were caught attesting
fraudulently could still contribute to a commit after revocation, defeating
the purpose of this mechanism.

**Voting-window expiry: round-based reset, not unbounded growth.** Each claim
carries a `round` counter. A vote arriving after `get_voting_window_secs()`
has elapsed since the round started bumps `round` and resets the tally to 1
(this vote), rather than accumulating forever — prior votes for the old round
become unreachable (their storage key is scoped to that round number) without
needing to enumerate or delete them. A validator whose vote was on an expired
round may vote again once a new round starts; their stale round-0 marker does
not block a fresh vote in round 1. The claim's storage record itself is
reused in place (not deleted), so it never becomes silently-unreachable dead
storage — `is_attestation_window_expired` reports its state explicitly.

**The off-chain-signed relay path is gated identically to `approve_milestone`.**
`submit_attested_milestone` (issue #703) commits a milestone via
`commit_approved_milestone` — the same shared commit function
`attest_milestone` uses at threshold — on the strength of exactly one
validator's ed25519 signature. Once `set_milestone_threshold(n)` with `n >= 2`
is configured, `submit_attested_milestone` also rejects every call with
`ThresholdModeRequiresAttestation`, exactly like `approve_milestone`. Without
this, k-of-n mode would eliminate the single-signature bypass through
`approve_milestone` while leaving an equivalent, unmonitored bypass open
through the relay path — a single validator's off-chain signature could still
unilaterally commit a milestone and trigger `progress.advance_level`, which
is precisely the trust model this mechanism exists to replace.

#### `attest_milestone(validator_wallet: Address, player_id: u64, description: String, evidence_hash: String) -> Result<AttestationStatus, VerificationError>`

Cast one independent vote toward a k-of-n threshold milestone claim. Returns
`AttestationStatus::Pending(vote_count)` if the claim is still short of
threshold, or `AttestationStatus::Committed(milestone_index)` if this vote
reached threshold and the milestone was committed (with
`progress.advance_level` cross-called, same as `approve_milestone`).

| | |
|---|---|
| **Auth** | `validator_wallet` must sign |
| **Errors** | `ContractPaused` · `ValidatorNotFound` · `ValidatorInactive` · `InvalidInput` · `DuplicateEvidence` (claim already committed) · `DuplicateAttestation` (same validator, same round) · `TooManyPendingVotes` (validator already has 25 concurrent open votes) · `Overflow` · `ProgressCallFailed` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- attest_milestone \
  --validator_wallet $VALIDATOR_ADDRESS \
  --player_id 1 \
  --description '"Scored 5 goals in Local Cup"' \
  --evidence_hash '"QmEvidence123"'
```

---

#### `set_milestone_threshold(threshold: u32) -> Result<(), VerificationError>` / `get_milestone_threshold() -> u32`

Configure (admin only) or read the k-of-n distinct-active-validator threshold
required before an `attest_milestone` claim commits. Must be in
`[1, MAX_VALIDATORS]`. Defaults to `1` — see `approve_milestone` above for why.
An already-open claim keeps the threshold in effect when its current round
started; changing this value only affects claims that start a fresh round
afterward, so the admin cannot retroactively fast-track or invalidate an
in-flight claim by moving the threshold mid-vote.

| | |
|---|---|
| **Auth** | admin must sign (`set_milestone_threshold` only) |
| **Errors** | `InvalidInput` (threshold is 0 or exceeds `MAX_VALIDATORS`) |

---

#### `set_voting_window_secs(window_secs: u64) -> Result<(), VerificationError>` / `get_voting_window_secs() -> u64`

Configure (admin only) or read the attestation voting window in seconds.
Must be in `[3_600, 7_776_000]` (1 hour – 90 days). Defaults to `1_209_600`
(14 days) — long enough for independently-transacting, geographically
distributed validators to notice and corroborate evidence; short enough that
a sub-threshold claim's fixed-size storage entry does not sit unresolved
indefinitely.

| | |
|---|---|
| **Auth** | admin must sign (`set_voting_window_secs` only) |
| **Errors** | `InvalidInput` (window outside the allowed range) |

---

#### `get_pending_claim(player_id: u64, evidence_hash: String) -> Option<PendingMilestoneClaim>`

Return the current accumulator state for a claim, if one is open. Returns
`None` once the claim commits (its storage is removed at that point) or
before any validator has attested to it. Includes `vote_count`, `round`,
`created_at`, and the `threshold` snapshotted when the current round started.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `has_attested(player_id: u64, evidence_hash: String, validator_wallet: Address) -> bool`

Whether `validator_wallet` has an active (not-yet-expired, not-yet-committed)
vote recorded for this claim's current round. Round-bumping on window expiry
is lazy — it only happens inside the next `attest_milestone` call for a given
claim — so this function independently re-checks the voting window itself
rather than trusting the claim's on-disk `round` field alone; it returns
`false` for a vote whose window has already elapsed even if no one has cast
the next vote yet to formally roll the round over.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `is_attestation_window_expired(player_id: u64, evidence_hash: String) -> bool`

Whether the claim's current voting round has exceeded the configured window
without reaching threshold. `true` means the next `attest_milestone` call for
this claim will start a fresh round rather than counting toward the existing
tally. Returns `false` when no claim is open.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `get_evidence_hash_usage(evidence_hash: String) -> Option<(u64, u32)>`

Return the original milestone consumer for an evidence hash that has already
been used by `approve_milestone`. Returns `Some((player_id, milestone_index))`
when the hash has been consumed, or `None` when it is still available for use.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_evidence_hash_usage \
  --evidence_hash '"QmEvidence123"'
```

---

#### `get_validators() -> Vec<Address>`

Return the list of all registered validator addresses (both active and revoked).
Capped at 100 addresses.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- get_validators
```

---

#### `get_validator_status(wallet: Address) -> ValidatorStatus`

Return the detailed status of a validator wallet: `Active`, `Revoked`, `RevokedForCause`, or
`NotRegistered`. Prefer this over `is_active_validator` for precise status
checks.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_status --wallet $VALIDATOR_ADDRESS
```

---

#### `get_validator_statuses(wallets: Vec<Address>) -> Vec<ValidatorStatus>`

Batch-fetch the status of up to 20 validator wallets in a single call.
Returns one `ValidatorStatus` entry per input wallet **in the same order as the input**,
including `NotRegistered` for wallets that have never been registered.

**Batch-size cap**: the first 20 entries are processed; wallets beyond that are silently
ignored. Call again with the remainder for larger sets. This is consistent with the 20-item
cap used by `registration.get_players`.

**Semantics**: unlike `registration.get_players` (which silently skips missing IDs), this
function always returns one entry per input wallet — including `NotRegistered` — because
`ValidatorStatus` already has a `NotRegistered` variant that makes the unregistered case
unambiguously representable. Callers always receive exactly N results for N inputs (up to the
cap), making it impossible to confuse "skipped" with "not registered".

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_statuses \
  --wallets '["$WALLET_1","$WALLET_2","$WALLET_3"]'
```

Compare with [`get_players`](#get_playersids-vecu64---resultvecplayersummary-scoutchainerror) in the registration contract for the equivalent batch-fetch pattern.

---

#### `get_validator_milestone_count(wallet: Address) -> u32`

Return the total number of milestones approved by a specific validator across
all players. Returns `0` for unregistered wallets.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_milestone_count --wallet $VALIDATOR_ADDRESS
```

---

#### `get_milestone(player_id: u64, index: u32) -> Result<Milestone, VerificationError>`

Read a specific milestone record. Indices start at `1`.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `MilestoneNotFound` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_milestone --player_id 1 --index 1
```

---

#### `get_milestone_with_validator_status(player_id: u64, index: u32) -> Result<MilestoneWithValidatorStatus, VerificationError>`

Read a specific milestone record along with the current status of the validator who approved it. Useful for checking if the approving validator was later revoked for cause.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `MilestoneNotFound` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_milestone_with_validator_status --player_id 1 --index 1
```

---

#### `get_milestone_count(player_id: u64) -> u32`

Return the total number of approved milestones for a player.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_milestone_count --player_id 1
```

---

#### `get_milestones_since(player_id: u64, since_timestamp: u64) -> Vec<Milestone>`

Return all milestones for a player where `approved_at >= since_timestamp`, in
approval order (oldest first).

This function mirrors [`progress.get_history_since`](#get_history_sinceplayerid-u64-sincetimestamp-u64---vecprogressentry)
in signature and semantics: an indexer that already tracks the timestamp of the
last milestone it processed can pass that timestamp to fetch only newly
approved milestones, avoiding a full re-fetch of the player's entire milestone
list on every sync cycle.

Returns an empty `Vec` when the player has no milestones, or when none satisfy
the timestamp predicate.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_milestones_since --player_id 1 --since_timestamp 1700000000
```

---

#### `get_validator(wallet: Address) -> Result<Validator, VerificationError>`

Read the full validator record including credentials, registration timestamp,
and active flag.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `ValidatorNotFound` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator --wallet $VALIDATOR_ADDRESS
```

---

#### `is_active_validator(wallet: Address) -> bool`

Boolean convenience check. Returns `true` only for registered, active
validators.

> **Deprecated** — use `get_validator_status` for precise `Active` / `Revoked` /
> `NotRegistered` disambiguation.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- is_active_validator --wallet $VALIDATOR_ADDRESS
```

---

#### `pause_contract() -> Result<(), VerificationError>`

Halt all state-changing operations. `approve_milestone` is blocked while paused.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- pause_contract
```

---

#### `unpause_contract() -> Result<(), VerificationError>`

Resume normal operations after a pause.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- unpause_contract
```

---

#### `upgrade(new_wasm_hash: BytesN<32>) -> Result<(), VerificationError>`

Upgrade the contract WASM to a new hash. Admin auth required. Persistent
storage (including the admin key) survives the upgrade.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- upgrade --new_wasm_hash $NEW_WASM_HASH
```

---

#### `get_total_milestone_count() -> u32`

Return the global total number of milestones approved across all players and
validators since contract initialization.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- get_total_milestone_count
```

---

#### `health() -> ContractHealth`

Return the contract's initialization and pause status.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- health
```

---

#### `upgrade(new_wasm_hash: BytesN<32>) -> Result<(), VerificationError>`

Replace the contract WASM in-place. Persistent storage (admin, validator registry, milestones) survives the upgrade. Instance storage (initialized flag, progress contract link) is retained but should be re-verified after the call.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NotInitialized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- upgrade --new_wasm_hash <NEW_WASM_HASH>
```

---

#### `get_total_milestone_count() -> u32`

Return the total number of milestones approved across all players and validators.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- get_total_milestone_count
```

---

#### `get_validator_players(wallet: Address) -> Vec<u64>`

Return all distinct player IDs for which `wallet` has approved at least one
milestone. Accumulated on every `approve_milestone` call; each player ID
appears at most once. This legacy method is unbounded; high-volume callers
should use `get_validator_players_page` instead.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_players --wallet $VALIDATOR_ADDRESS
```

---

#### `get_validator_players_page(wallet: Address, offset: u32, limit: u32) -> ValidatorPlayersPage`

Return a bounded, paginated page of distinct player IDs for which the validator
has approved at least one milestone, together with the total number of distinct
players.

This is the canonical paginated successor to the unbounded `get_validator_players`.
The `total` field lets callers determine when paging is complete without
over-fetching.

**Pagination**: `offset` is a zero-based item offset; `limit` is capped at 50 entries
per page, matching `get_global_milestone_index`. Returns an empty `entries` vec
when the offset is beyond the validator's player list.

**Ordering**: entries are returned in order of first approval.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_players_page --wallet $VALIDATOR_ADDRESS --offset 0 --limit 50
```

---

#### `get_validator_activity_report(wallet: Address) -> Result<ValidatorActivityReport, VerificationError>`

Convenience aggregate query — bundles the data from four individual queries into
one call, reducing round-trips for admin dashboards and monitoring tools.

Internally aggregates exactly:
1. `get_validator(wallet)` → `credentials`, `registered_at`, `active`
2. `get_validator_status(wallet)` → `status`
3. `get_validator_milestone_count(wallet)` → `milestone_count`
4. `get_validator_players(wallet)` → `distinct_players` (and `distinct_player_count`)

This is a **pure read-only aggregation** — no new storage, no new business logic.
The returned values are byte-for-byte identical to calling the four individual
queries separately.

Returns `ValidatorNotFound` if the wallet has never been registered.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `ValidatorNotFound` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_activity_report --wallet $VALIDATOR_ADDRESS
```

---

#### `get_active_validator_count() -> u32`

Return the number of currently active (non-revoked) validators.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- get_active_validator_count
```

---

#### `get_validator_count() -> u32`

Return the total number of registered validators (both active and revoked).
Useful as a pre-check before calling `register_validator` to anticipate a
possible `ValidatorCapReached` error, since the validator registry is capped at
100 addresses total.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- get_validator_count
```

---

#### `get_active_disputes_count() -> u32`

Return the number of currently active (unresolved) disputes across all
players and milestones. The count is incremented on every `dispute_milestone`
call and decremented when `resolve_dispute` marks a dispute resolved.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- get_active_disputes_count
```

---

#### `list_disputes_page(offset: u32, limit: u32) -> Vec<(u64, u32)>`

Return a bounded, paginated page of currently-unresolved
`(player_id, milestone_index)` dispute keys, platform-wide.

The underlying index (`DataKey::OpenDisputeIndex`) is maintained at write-time:
`dispute_milestone` appends an entry when a new dispute is filed, and
`resolve_dispute` removes it when the dispute is resolved. This means the index
always reflects exactly the set of open disputes with no full-scan required at
query time — making it possible to build an admin "disputes needing attention"
dashboard from on-chain queries alone.

- `limit` is capped at **50** per page (minimum 1), consistent with
  `get_global_milestone_index` and `get_validator_milestones_page`.
- `offset` is a zero-based item offset (e.g. `offset=0, limit=50` → first page;
  `offset=50, limit=50` → second page). If `offset >= total`, an empty list is
  returned immediately without iterating the index.
- Entries are returned **oldest-first** (insertion order).
- The index tracks **only unresolved disputes** — resolved disputes are removed
  immediately, so the index stays naturally bounded in size.

Use `get_active_disputes_count` to get the total count for building pagination UI
without fetching the full list.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
# First page of open disputes (up to 50)
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- list_disputes_page --offset 0 --limit 50

# Second page
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- list_disputes_page --offset 50 --limit 50
```

---

#### `get_global_milestone_index(offset: u32, limit: u32) -> GlobalMilestoneIndexPage`

Return a page of the global milestone index — a rolling log of the most
recent `(player_id, milestone_index)` pairs across all players and validators
(capped at 500 entries; oldest entries are evicted first). `limit` is capped
at 50 entries per page (minimum 1). `GlobalMilestoneIndexPage` has `entries:
Vec<GlobalMilestoneEntry>` and `total: u32`.

If `offset >= total`, `entries` is empty and `total` still reflects the full
index length — safe to use for pagination bounds checks.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_global_milestone_index --offset 0 --limit 50
```

---

#### `get_milestones_since_page(player_id: u64, since_timestamp: u64, offset: u32, limit: u32) -> Vec<Milestone>`

Return a bounded page of milestones for `player_id` that were approved at or
after `since_timestamp` (Unix seconds). This is the bounded replacement for
an unbounded time-range scan.

**Pagination contract:**
- `limit` is capped at **50** per page (minimum 1).
- `offset` is bounded against the player's milestone count: if `offset >=
  count`, an empty list is returned immediately without iterating.
- Results are returned in approval order (oldest first within the page).
- Callers who want all milestones without a time filter should use
  `get_milestone_count` + `get_milestone` directly.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
# Milestones for player 1 approved after a given Unix timestamp
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_milestones_since_page --player_id 1 --since_timestamp 1700000000 \
     --offset 0 --limit 50
```

---

#### `get_validator_milestones(wallet: Address) -> Vec<MilestoneRef>`

Return the list of `(player_id, milestone_index)` references for every
milestone `wallet` has approved. `MilestoneRef` has `player_id: u64` and
`milestone_index: u32`. This legacy method is unbounded; high-volume callers
should use `get_validator_milestones_page_v2` instead.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_milestones --wallet $VALIDATOR_ADDRESS
```

---

#### `get_validator_milestones_page(wallet: Address, offset: u32, limit: u32) -> Vec<MilestoneRef>`

> **Deprecated**: use `get_validator_milestones_page_v2` which returns a structured `MilestoneRefPage`
> with `entries` and `total` fields instead of a raw `Vec`.

Return a bounded page of `(player_id, milestone_index)` references for milestones
approved by `wallet`. `offset` is zero-based and `limit` is capped at 50 entries,
matching `get_global_milestone_index`. Returns an empty `Vec` when the offset is
beyond the validator's approval history or `limit` is zero.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_milestones_page --wallet $VALIDATOR_ADDRESS --offset 0 --limit 50
```

---

#### `get_validator_milestones_page_v2(wallet: Address, offset: u32, limit: u32) -> MilestoneRefPage`

Return a bounded, paginated page of `(player_id, milestone_index)` references for
milestones approved by `wallet`, together with the total number of milestones.

This is the canonical successor to both `get_validator_milestones` (unbounded,
deprecated) and `get_validator_milestones_page` (returns raw Vec). The `total` field
lets callers determine when paging is complete without over-fetching.

**Pagination**: `offset` is a zero-based item offset; `limit` is capped at 50 entries
per page, matching `get_global_milestone_index`. Returns an empty `entries` vec
when the offset is beyond the validator's approval history.

**Ordering**: entries are returned in approval order (oldest first).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_validator_milestones_page_v2 --wallet $VALIDATOR_ADDRESS --offset 0 --limit 50
```

---

#### `dispute_milestone(player_wallet: Address, player_id: u64, milestone_index: u32, reason: String, impact_score: u32) -> Result<(), VerificationError>`

Allow a player to dispute a milestone they believe was wrongly attributed.
Only the player associated with `player_id` may submit a dispute. The
`impact_score` parameter routes the dispute: if `impact_score >= jury_config.impact_threshold`
the dispute is jury-required and must be finalized via `tally_dispute`; otherwise
the existing admin-only `resolve_dispute` path applies.

The current `JuryConfig` (quorum and voting window) is **snapshotted** at
filing time — later calls to `set_jury_config` cannot alter an in-progress dispute's
rules. A new dispute is stored as `resolved: false`, `upheld: false`, with
`votes_for: 0` and `votes_against: 0`. Only one dispute record may exist per
`(player_id, milestone_index)` pair. Emits a `milestone_disputed` event.

Authorization works in two steps:
1. `player_wallet.require_auth()` proves the caller controls the claimed wallet.
2. A cross-contract call to the **registration contract** (`get_player(player_id)`)
   verifies that `profile.wallet == player_wallet`, binding the wallet to the
   player ID.

| | |
|---|---|
| **Auth** | `player_wallet` must sign, and must match the wallet on record for `player_id` in the registration contract |
| **Errors** | `ContractPaused` · `NotInitialized` · `MilestoneNotFound` · `Unauthorized` · `InvalidInput` (dispute already exists) · `RegistrationCallFailed` · `Overflow` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- dispute_milestone \
  --player_wallet $PLAYER_ADDRESS \
  --player_id 1 \
  --milestone_index 1 \
  --reason '"Milestone not actually completed"' \
  --impact_score 120
```

---

#### `set_jury_config(impact_threshold: u32, quorum: u32, voting_window_secs: u64) -> Result<(), VerificationError>`

Admin-only. Configure the jury escalation parameters. Changes only affect
disputes filed **after** this call — in-flight disputes keep their snapshotted
values.

| Parameter | Default | Description |
|-----------|---------|-------------|
| `impact_threshold` | 100 | Disputes with `impact_score >= threshold` are jury-routed |
| `quorum` | 3 | Minimum distinct validator votes required for a jury outcome |
| `voting_window_secs` | 604800 | Seconds after filing when the voting window closes (7 days) |

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ContractPaused` · `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- set_jury_config \
  --impact_threshold 100 \
  --quorum 3 \
  --voting_window_secs 604800
```

---

#### `get_jury_config() -> JuryConfig`

Return the current `JuryConfig` (`impact_threshold`, `quorum`, `voting_window_secs`).
Returns defaults (100 / 3 / 604800) if `set_jury_config` has never been called.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- get_jury_config
```

---

#### `cast_dispute_vote(validator: Address, player_id: u64, milestone_index: u32, for_upheld: bool) -> Result<(), VerificationError>`

Cast a validator vote on a jury-required milestone dispute. All four eligibility
rules must hold:

1. Wallet is a registered **active** validator.
2. Validator is **not** the original approver of the disputed milestone (conflict of interest).
3. Validator has **not** already voted on this dispute.
4. Dispute is jury-required, unresolved, and the voting window is still open.

Vote tallies (`votes_for` / `votes_against`) are updated atomically on the
dispute record. An individual `DisputeVote` record (with `voted_at` timestamp)
is written for audit trail. Emits `dispute_vote_cast`.

| | |
|---|---|
| **Auth** | `validator` must sign |
| **Errors** | `ContractPaused` · `NotInitialized` · `ValidatorNotFound` · `ValidatorInactive` · `MilestoneNotFound` · `NotJuryDispute` · `DisputeAlreadyResolved` · `VotingWindowClosed` · `ConflictOfInterest` · `AlreadyVoted` · `Overflow` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- cast_dispute_vote \
  --validator $VALIDATOR_ADDRESS \
  --player_id 1 \
  --milestone_index 1 \
  --for_upheld true
```

---

#### `tally_dispute(player_id: u64, milestone_index: u32) -> Result<(), VerificationError>`

Finalize a jury-required milestone dispute. Callable by anyone (no admin required).
Succeeds when the dispute is jury-required, unresolved, and either:

- **Early close**: total votes ≥ quorum **and** `votes_for ≠ votes_against` (clear majority), or
- **Deadline passed**: current ledger timestamp ≥ `voting_deadline`.

**Outcome rules:**

| Condition | `upheld` |
|-----------|---------|
| Below quorum at deadline | `false` |
| `votes_for > votes_against` | `true` |
| `votes_against > votes_for` | `false` |
| Tie (`votes_for == votes_against`) | `false` (tie-break: reject) |

Marks the dispute resolved, decrements the active-disputes counter, removes
the dispute from the open-dispute index, and emits `dispute_tallied`.
Does **not** roll back player progress (same semantics as `resolve_dispute`).

| | |
|---|---|
| **Auth** | None (callable by anyone) |
| **Errors** | `ContractPaused` · `NotInitialized` · `MilestoneNotFound` · `NotJuryDispute` · `DisputeAlreadyResolved` · `VotingWindowOpen` (tied at quorum, window open) · `QuorumNotReached` (below quorum, window open) · `Overflow` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- tally_dispute --player_id 1 --milestone_index 1
```

---

#### `resolve_dispute(player_id: u64, milestone_index: u32, upheld: bool) -> Result<(), VerificationError>`

Admin-only review action for a filed milestone dispute. Marks the stored
`MilestoneDispute` as `resolved: true`, records the admin's outcome in `upheld`,
decrements `get_active_disputes_count()`, and emits a `dispute_resolved` event.
This function deliberately does not roll back player progress when `upheld` is
true; that corrective workflow is tracked separately.

**Returns `DisputeRequiresJury`** for disputes with `jury_required == true` —
those must be finalized via `tally_dispute`, not by admin.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ContractPaused` · `NotInitialized` · `Unauthorized` · `MilestoneNotFound` (no dispute recorded) · `DisputeAlreadyResolved` · `DisputeRequiresJury` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- resolve_dispute --player_id 1 --milestone_index 1 --upheld false
```

---

#### `get_dispute(player_id: u64, milestone_index: u32) -> Result<MilestoneDispute, VerificationError>`

Read a milestone dispute by `(player_id, milestone_index)`. `MilestoneDispute`
has `player_id: u64`, `milestone_index: u32`, `reason: String`,
`disputed_at: u64`, `resolved: bool`, `upheld: bool`, `impact_score: u32`,
`jury_required: bool`, `quorum: u32`, `voting_deadline: u64`,
`votes_for: u32`, and `votes_against: u32`.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `MilestoneNotFound` (no dispute recorded) |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_dispute --player_id 1 --milestone_index 1
```

---

#### `has_dispute(player_id: u64, milestone_index: u32) -> bool`

Boolean convenience check. Returns `true` if a dispute exists for the given
`(player_id, milestone_index)` pair, `false` otherwise (including when no
dispute has ever been submitted or the milestone itself does not exist).

This is a thin read-only wrapper around `get_dispute` — no new storage is
introduced. Mirrors the `is_active_validator` pattern: callers that only need
a yes/no answer (e.g. a frontend showing a "disputed" badge next to a milestone)
avoid handling a `Result`/error path.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- has_dispute --player_id 1 --milestone_index 1
```

---

#### `get_player_dispute_count(player_id: u64) -> u32`

Return the total number of disputes filed for a given `player_id`.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_player_dispute_count --player_id 1
```

---

#### `get_player_disputes(player_id: u64, offset: u32, limit: u32) -> Vec<MilestoneDispute>`

Return a paginated list of all milestone disputes filed for `player_id`.
`offset` is zero-based and `limit` is capped at 50 entries.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_player_disputes --player_id 1 --offset 0 --limit 50
```

---

#### `get_player_disputes_by_status(player_id: u64, resolved: bool, offset: u32, limit: u32) -> Vec<MilestoneDispute>`

Return a paginated list of milestone disputes for `player_id` filtered by resolution status.
If `resolved` is `true`, only resolved disputes are returned. If `resolved` is `false`, only open/unresolved disputes are returned.
`limit` is capped at 50 entries.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- get_player_disputes_by_status --player_id 1 --resolved false --offset 0 --limit 50
```

---

#### `version() -> String`

Return the deployed contract version string (from `Cargo.toml` at build time).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- version
```

---

### Events

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `contract_initialized` | event_name, admin (Address) | admin (Address) | Emitted on successful initialization |
| `admin_transfer_proposed` | event_name, old_admin (Address) | new_admin (Address) | Admin replacement proposed |
| `admin_transferred` | event_name, old_admin (Address) | new_admin (Address) | Pending admin accepts control |
| `milestone_approved` | event_name, validator (Address) | player_id (u64), milestone_index (u32), description (String), evidence_hash (String) | Validator confirms a player achievement |
| `validator_registered` | event_name, wallet (Address) | credentials (String) | New validator onboarded |
| `validator_revoked` | event_name, admin (Address) | wallet (Address), reason (String) | Validator deactivated |
| `validator_restored` | event_name, admin (Address) | wallet (Address) | Revoked validator re-activated |
| `validator_transferred` | event_name, admin (Address) | old_wallet (Address), new_wallet (Address) | Validator identity migrated to new wallet |
| `milestone_disputed` | event_name, player_wallet (Address) | player_id (u64), milestone_index (u32), reason (String) | Player disputes a milestone attribution |
| `dispute_resolved` | event_name, admin (Address) | player_id (u64), milestone_index (u32), upheld (bool) | Admin resolves a milestone dispute |
| `progress_contract_updated` | event_name, admin (Address) | progress_contract (Address) | Progress contract re-wired |
| `contract_paused` | event_name, admin (Address) | () | Circuit breaker engaged |
| `contract_unpaused` | event_name, admin (Address) | () | Circuit breaker released |

#### Diagnostic Events (verification)

The following events are emitted for observability when level advancement is skipped or fails. They allow the off-chain indexer to detect silent failures without scanning every transaction receipt for error codes.

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `level_advancement_skipped` | event_name, player_id (u64) | reason (String) | Milestone recorded but level not advanced because player is already at `EliteTier`. `reason` is always `"AlreadyAtMaxLevel"`. Committed to the ledger. |
| `progress_contract_not_set` | event_name, player_id (u64) | `()` | Level advancement skipped because the progress contract address has not been configured. Indicates missing wiring — alert in production. Committed to the ledger. |
| `progress_call_failed` | event_name, player_id (u64) | error_code (u32) | Emitted just before `ProgressCallFailed` is returned. Because that error aborts the entire transaction, this event only appears in the **diagnostic stream** (transaction receipt), not in committed ledger events. `error_code` is the raw error discriminant from `try_advance_level`. |

---

## progress

`ProgressEntry.updated_at` and the `since_timestamp` parameter are Unix
seconds. `ProgressEntry.ledger_sequence` is instead a Soroban ledger sequence
number, not a timestamp. See [Timestamp](GLOSSARY.md#timestamp).

### Functions

---

#### `initialize(admin: Address) -> Result<(), ProgressError>`

One-time contract setup.

| | |
|---|---|
| **Auth** | `admin` must sign |
| **Errors** | `AlreadyInitialized` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- initialize --admin $ADMIN_ADDRESS
```

---

#### `propose_admin(new_admin: Address) -> Result<(), ProgressError>`

Store or replace a pending admin proposal. The current admin retains all
privileges until the proposed address accepts.

| | |
|---|---|
| **Auth** | Current admin must sign |
| **Errors** | `NotInitialized` |
| **Emits** | `admin_transfer_proposed` with `(old_admin, new_admin)` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- propose_admin --new_admin $NEW_ADMIN_ADDRESS
```

---

#### `accept_admin() -> Result<(), ProgressError>`

Finalize the transfer. The stored pending admin must sign, proving control of
the address. Acceptance updates the admin and clears the proposal.

| | |
|---|---|
| **Auth** | Pending admin must sign |
| **Errors** | `NotInitialized` · `PendingAdminNotSet` |
| **Emits** | `admin_transferred` with `(old_admin, new_admin)` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID -- accept_admin
```

---

#### `transfer_admin(new_admin: Address) -> Result<(), ProgressError>`

Deprecated compatibility alias for `propose_admin`. It creates or replaces a
proposal and does not immediately change the admin.

---

#### `reset_player_level(player_id: u64, target_level: ProgressLevel) -> Result<(), ProgressError>`

Reset a player's progress level for dispute resolution or correction.
Existing history is preserved; a new `ProgressEntry` recording the reset is
appended. `milestone_ref` is `0` for admin resets.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` · `ContractPaused` · `Overflow` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- reset_player_level \
  --player_id 1 \
  --target_level '"Unverified"'
```

---

#### `advance_level(caller: Address, player_id: u64, milestone_ref: u32) -> Result<ProgressLevel, ProgressError>`

Advance a player's progress level by one tier. `milestone_ref` links back to
the verification contract's milestone index. Returns the new `ProgressLevel`.

Only the configured `VerificationContract` (primary) or `ScoutAccessContract`
(secondary, for trial-offer Level-3 advances) may invoke this function. There is
**no open fallback**: if neither address is configured the call is rejected with
`NotInitialized`. Auth is required from the *stored* whitelist address — the
`caller` argument is recorded as `updated_by` in the history entry but is not
itself the authorising party.

On the secondary path, `milestone_ref` must be backed by a real milestone in the
verification contract (`0 < milestone_ref <= get_milestone_count(player_id)`),
otherwise the call fails with `InvalidProgressTransition` (#457).

| | |
|---|---|
| **Auth** | Configured verification contract (primary) or scout_access contract (secondary) |
| **Errors** | `NotInitialized` · `ContractPaused` · `InvalidProgressTransition` · `AlreadyAtMaxLevel` · `Overflow` · `RegistrationCallFailed` |

_Called atomically by `verification.approve_milestone`. Prefer that path in production._

> **Not idempotent — do not retry this call directly (Issue #811 follow-up).**
> `advance_level` is a monotonic state-machine step, not a keyed mutation: it
> reads the current level, computes `next()`, and appends a history entry. It
> does **not** deduplicate on `milestone_ref`, so two calls with the *same*
> `milestone_ref` advance two tiers and append two history entries that are
> indistinguishable from a legitimate double advance.
>
> Retry-after-uncertain-failure is safe only via the production entry points,
> which each hold their own dedup key — `verification.approve_milestone` uses
> `EvidenceUsed(evidence_hash)` (`DuplicateEvidence`), and
> `scout_access.confirm_trial_offer` uses `ConfirmationNonce`
> (`TrialOfferAlreadyConfirmed`). Because a failed attempt reverts the whole
> transaction, the dedup key is rolled back with it and the retry applies
> exactly once.
>
> Any new whitelisted caller **must** supply its own dedup key. Replay exposure
> is bounded to three tiers: at `EliteTier` further calls fail closed with
> `AlreadyAtMaxLevel` and write nothing. See
> `contracts/progress/tests/issue_811_idempotency.rs`.

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- advance_level \
  --caller $VALIDATOR_ADDRESS \
  --player_id 1 \
  --milestone_ref 1
```

---

#### `get_level(player_id: u64) -> ProgressLevel`

Return the player's current progress level. Returns `Unverified` for unknown
player IDs (no `PlayerNotFound` error) — a **default-on-absent** getter.

Reading is a keep-alive: when a `PlayerLevel` record exists, `get_level` extends
its TTL so a dormant player's level is not lost to archival decay. The extension
is skipped when no record exists — extending the TTL of an unwritten key raises
`Storage/MissingValue`, which the host escalates to a panic, so an unguarded
extension would make the documented `Unverified` default trap instead of return.

The `Unverified` default is indistinguishable from a player genuinely stuck at
the base tier, so this getter alone cannot assert a player's existence. To
confirm a player exists, check the registration contract
(`registration.get_player(player_id)`, which returns `PlayerNotFound` for an
unknown ID) rather than reading it off the level.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None — never traps, including for unknown player IDs |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_level --player_id 1
```

---

#### `get_history_count(player_id: u64) -> u32`

Return the total number of history entries recorded for a player. **Returns `0`
on absent storage — for a registered player with no recorded level changes and
for an *unknown* `player_id` alike — with no error (default-on-absent).**

There is no distinct empty-vs-missing signal: a `0` cannot tell you whether the
player exists. Do not use this getter to assert existence; confirm the player
against the registration contract (`registration.get_player(player_id)`, which
returns `PlayerNotFound` for an unknown ID) and only then treat a `0` here as
"no history entries." See [`docs/INDEXER.md`](INDEXER.md) for how the
`player_level_history` reconciliation cross-check relies on this disambiguation.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None — returns the `0` default on absent storage |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_history_count --player_id 1
```

---

#### `get_history_entry(player_id: u64, index: u32) -> Result<ProgressEntry, ProgressError>`

Read a specific history entry. Indices start at `1`. Each `ProgressEntry`
includes `updated_at` in Unix seconds and `ledger_sequence: u32`, the Soroban
ledger sequence number at the time of the change (not a timestamp), for
tamper-proof auditability.

Unlike the count / list getters above, this one is **not** default-on-absent:
an out-of-range `index` (including any index on an unknown player, so `0`
entries) fails with `PlayerNotFound` rather than returning a zero value. This
makes it the only progress getter that can itself reject an unknown player.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `PlayerNotFound` (index out of range) |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_history_entry --player_id 1 --index 1
```

---

#### `get_progress_history(player_id: u64) -> Vec<ProgressEntry>`

Return all history entries for a player in chronological order. The contract now
stores the full logical history as bounded `HistoryPage(player_id, page_index)`
shards (fixed-size pages, not one ever-growing `HistoryVec` key) and
reconstructs the chronological list at read time. This keeps per-entry storage
cost bounded even if a player experiences many resets or repeated re-entries.
Returns an empty `Vec` for unknown player IDs (default-on-absent, no error).
Because an empty result is returned both for a registered player with no history
and for an unknown `player_id`, verify existence against the registration
contract (`registration.get_player(player_id)`) before interpreting the empty
list, and distinguish "no level changes" from "player unknown" via that call.

**Gas trade-off**: each page is a small, fixed-size `Vec<ProgressEntry>`, so the
read cost scales with the number of pages touched rather than the total lifetime
entry count in a single unbounded storage key. The logical history can still be
reconstructed for Merkle commitments and auditing without exposing an
unbounded per-player storage blob.

**Migration note**: the legacy `HistoryVec(player_id)` key remains readable for
compatibility with older deployments and recovery tooling, but new writes append
to `HistoryPage` shards instead of extending the legacy vec. Existing data can
still be recovered by concatenating the `HistoryEntry(player_id, i)` records in
index order until a one-time migration is complete.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_progress_history --player_id 1
```

---

#### `pause_contract() -> Result<(), ProgressError>`

Halt all state-changing operations.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID -- pause_contract
```

---

#### `unpause_contract() -> Result<(), ProgressError>`

Resume normal operations after a pause.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID -- unpause_contract
```

---

#### `health() -> ContractHealth`

Return the contract's initialization and pause status.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID -- health
```

---

#### `set_verification_contract(addr: Address) -> Result<(), ProgressError>`

Store the verification contract address so `advance_level` can authenticate cross-contract callers. Without this, only direct `caller` auth is accepted (useful for testing). Admin only. Freely re-settable — no guard. Bumps `VerificationContract`'s re-wiring epoch and emits `wiring_updated` (`link = "verification_contract"`) on every call (issue #1041).

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- set_verification_contract --addr $VERIFICATION_CONTRACT_ID
```

---

#### `set_registration_contract(addr: Address) -> Result<(), ProgressError>`

Store the registration contract address so `advance_level` can sync player levels via cross-contract call. Admin only. Freely re-settable — no guard. Bumps `RegistrationContract`'s re-wiring epoch and emits `wiring_updated` (`link = "registration_contract"`) on every call.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- set_registration_contract --addr $REGISTRATION_CONTRACT_ID
```

---

#### `set_scout_access_contract(addr: Address) -> Result<(), ProgressError>`

Whitelist the scout_access contract as a secondary authorized caller of `advance_level` (for trial-offer Level-3 advances). Admin only. Freely re-settable — no guard. Bumps `ScoutAccessContract`'s re-wiring epoch and emits `wiring_updated` (`link = "scout_access_contract"`) on every call.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- set_scout_access_contract --addr $SCOUT_ACCESS_CONTRACT_ID
```

---

#### `get_verification_contract() -> Option<Address>`

Returns the configured verification contract address, or `None` when unset.
Read-only and requires no auth.

#### `get_registration_contract() -> Option<Address>`

Returns the configured registration contract address, or `None` when unset.
Read-only and requires no auth.

#### `get_scout_access_contract() -> Option<Address>`

Returns the configured scout_access contract address, or `None` when unset.
Read-only and requires no auth.

---

#### `get_wiring_state() -> ProgressWiringState`

Returns a snapshot of all three peer-address pointers this contract holds (`registration_contract`, `verification_contract`, `scout_access_contract`), each paired with a re-wiring epoch (`registration_epoch`, `verification_epoch`, `scout_access_epoch` — `0` iff the corresponding address is `None`). `ProgressWiringState::is_fully_wired()` returns `true` iff all three addresses are set. Read-only, no auth required, exempt from the pause/init guards so it stays callable on a mis-wired or paused contract — see [`docs/WIRING_REGISTRY_DESIGN.md`](WIRING_REGISTRY_DESIGN.md).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_wiring_state
```

---

#### `upgrade(new_wasm_hash: BytesN<32>) -> Result<(), ProgressError>`

Replace the contract WASM in-place. Persistent storage (admin, history) survives the upgrade. Admin only.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NotInitialized` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- upgrade --new_wasm_hash <NEW_WASM_HASH>
```

---

#### `get_progress_history_page(player_id: u64, offset: u32, limit: u32) -> Vec<ProgressEntry>`

Paginated history retrieval. Returns entries starting at `offset+1`. `limit` is clamped to the range 1 through 50. Returns an empty `Vec` when `offset` >= total count, **and also returns an empty `Vec` for an unknown `player_id` (default-on-absent, no error)** — an unknown player has a count of `0`, so `offset` is always `>= count`. An empty result therefore does not by itself prove the player exists; confirm existence against the registration contract (`registration.get_player(player_id)`).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_progress_history_page --player_id 1 --offset 0 --limit 10
```

---

#### `get_history_since(player_id: u64, since_timestamp: u64) -> Vec<ProgressEntry>`

Return all of a player's history entries with `updated_at >= since_timestamp`
(Unix seconds). Useful for indexers polling for changes since their last sync
point instead of re-reading the full history.

Returns an empty `Vec` for an unknown `player_id` (default-on-absent, no error)
— the same empty result a registered player with no matching entries yields —
so this getter does not assert existence. Confirm the player against the
registration contract (`registration.get_player(player_id)`) when the empty
result's cause matters.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_history_since --player_id 1 --since_timestamp 1700000000
```

---

### Merkle history commitment

`get_progress_history` and friends are only as trustworthy as the RPC node
that answers the query — nothing lets a caller independently check that a
returned `ProgressEntry` is genuinely part of the on-chain record without
re-trusting that same node. `get_progress_root` / `verify_history_proof`
close that gap: `record_progress_entry` maintains a cryptographic
commitment (a Merkle root) over each player's full history, computed with
`env.crypto().sha256()` — the only hash primitive `soroban-sdk` 25.3.1
exposes — and any caller can verify an arbitrary historical entry against
the current root using a proof, entirely on-chain, without trusting
whichever node served the entry or the proof.

**Construction: RFC 6962 Merkle Tree Hash, recomputed on append, not an
MMR.** An incremental accumulator (Merkle Mountain Range or similar) is the
natural shape for a log with unbounded, expensive-to-reread history. This
one is neither: `record_progress_entry` already reads and rewrites the
player's full `HistoryVec` on every append for an unrelated, pre-existing
reason (`get_progress_history`'s O(1)-read optimization), so the complete
leaf list is already materialized in memory at zero extra storage I/O.
Recomputing the [RFC 6962](https://www.rfc-editor.org/rfc/rfc6962) Merkle
Tree Hash — a standard, formally specified, domain-separated binary tree
construction that (unlike a naive Bitcoin-style pairwise tree) is well
defined for any number of leaves, not just powers of two — from that
already-materialized list costs `O(n)` extra `sha256` calls with no extra
storage operations. `n` is bounded in practice: three tier advances plus
any admin dispute-resolution resets via `reset_player_level`, not an
unbounded log, so this stays cheap while being simpler to get correct than
maintaining a separate incremental peaks structure. See
`ci/cpu-cost-budget.md` for the measured cost this adds to `advance_level`.

Leaf hashes are `H(0x00 || player_id || old_level_code || new_level_code ||
updated_by.to_xdr() || updated_at || milestone_ref || ledger_sequence)` in
that fixed field order (`level_code` is a stable 0–3 tier code, not the
Rust enum discriminant); internal nodes are `H(0x01 || left || right)`. The
`0x00`/`0x01` prefixes domain-separate leaves from internal nodes, closing
the classic second-preimage attack against naive Merkle trees (replaying an
internal node's hash as if it were a leaf, or vice versa).

**Proof format and bounded verification cost.** A proof is a `Vec` of
`(sibling_hash, sibling_is_right)` steps, ordered leaf-to-root — the RFC
6962 audit path. `verify_history_proof` never panics on bad input: it
rejects proofs longer than a fixed step cap outright (bounding adversarial
verification cost regardless of what a caller submits) and otherwise simply
replays the hash chain, which for any malformed, forged, or stale proof
produces some 32-byte value that will not equal the stored root, returning
`Ok(false)` rather than erring.

#### `get_progress_root(player_id: u64) -> BytesN<32>`

Return the current Merkle commitment root for a player's history. Returns
32 zero bytes for a player with no recorded history (never a valid
commitment for real history, and `verify_history_proof` never treats it as
one — see that function's `PlayerNotFound` case).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_progress_root --player_id 1
```

---

#### `get_history_proof(player_id: u64, index: u32) -> Result<Vec<HistoryProofStep>, ProgressError>`

Generate a Merkle inclusion proof for the history entry at `index`
(1-indexed, matching `get_history_entry`), valid against the player's
*current* `get_progress_root`. A convenience for callers who would
otherwise have to reimplement the tree construction off-chain — recomputed
on demand, not stored (storing a proof per entry would require rewriting
every prior entry's proof on each append). `verify_history_proof` accepts
proofs from any source, not only this function.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `PlayerNotFound` (no history, or `index` is `0` or beyond the entry count) |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_history_proof --player_id 1 --index 1
```

---

#### `verify_history_proof(player_id: u64, entry: ProgressEntry, proof: Vec<HistoryProofStep>) -> Result<bool, ProgressError>`

Verify that `entry` is genuinely committed in `player_id`'s history at the
current root, using a caller-supplied Merkle proof. This is the
independently-checkable half of the tamper-proof-history guarantee: the
result is a pure function of `(player_id, entry, proof, stored_root)`
computed entirely on-chain, so a caller does not need to trust the RPC node
answering the call — it can be cross-checked against multiple nodes, or the
caller can supply a proof it derived itself from `get_progress_history`.

Returns `Ok(false)` — never panics — for a forged entry, a proof computed
against a stale root (e.g. one predating the player's most recent append),
or a structurally malformed proof (wrong length, empty when steps are
required, or longer than the fixed step cap). Returns
`Err(ProgressError::PlayerNotFound)` only when the player has no committed
root at all — there is nothing to verify against, a different condition
from an existing player's proof simply failing to verify.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `PlayerNotFound` (player has no history at all) |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- verify_history_proof --player_id 1 \
  --entry '<ProgressEntry JSON>' --proof '<HistoryProofStep[] JSON>'
```

---

#### `version() -> String`

Return the deployed contract version string (from `Cargo.toml` at build time).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID -- version
```

### Events

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `progress_updated` | event_name, updated_by (Address) | player_id (u64), old_level, new_level | Player advances one tier |
| `player_level_reset` | event_name, admin (Address) | player_id (u64), old_level, new_level | Admin resets a player's level |
| `admin_transfer_proposed` | event_name, old_admin (Address) | new_admin (Address) | Admin replacement proposed |
| `admin_transferred` | event_name, old_admin (Address) | new_admin (Address) | Admin rights rotated |
| `contract_paused` | event_name, admin (Address) | () | Circuit breaker engaged |
| `contract_unpaused` | event_name, admin (Address) | () | Circuit breaker released |

### ProgressError Codes

| Code | Error | Description |
|------|-------|-------------|
| 1 | `AlreadyInitialized` | Contract has already been initialized |
| 2 | `NotInitialized` | Contract has not been initialized yet |
| 3 | `ContractPaused` | Contract is paused by the emergency circuit breaker |
| 4 | `Unauthorized` | Caller is not authorized for the requested operation |
| 5 | `InvalidProgressTransition` | Requested level transition is not allowed |
| 6 | `AlreadyAtMaxLevel` | Player is already at the maximum progress level (EliteTier) |
| 7 | `PlayerNotFound` | Player ID does not exist in progress storage |
| 8 | `HistoryNotFound` | Progress history record does not exist for this player |
| 9 | `InvalidHistoryEntry` | History entry data is malformed or inconsistent |
| 10 | `ProgressRecordEvicted` | Progress record was evicted from contract storage |
| 11 | `MigrationNotActive` | Migration operation attempted when no migration is in progress |
| 12 | `HistoryAlreadyExists` | History entry for this level already exists for the player |
| 13 | `MerkleRootMismatch` | Provided Merkle root does not match the stored root |
| 14 | `InvalidHistoryIndex` | Requested history index is out of bounds |
| 15 | `PlayerLevelRecordEvicted` | Player level record was evicted from contract storage |

---

## scout_access

Handles scout subscriptions, pay-to-contact flows, and trial offer logging.
Fees are collected in XLM (stroops) and held in the contract until admin
withdrawal.

Absolute timestamp fields returned by this contract (`expires_at`,
`subscribed_at`, `contacted_at`, `logged_at`, and `period_start`) are Unix
seconds. `sub_duration_secs` is a duration in seconds, not a Unix timestamp.
See [Timestamp](GLOSSARY.md#timestamp).

### `FeeConfig` Struct

Primary configuration struct controlling all subscription and contact fees.
Passed to `initialize` and `update_fee_config`. All fields must be strictly
greater than zero; either function returns `InvalidInput` otherwise.

| Field | Rust Type | Unit | Valid Range | Typical Example |
|---|---|---|---|---|
| `contact_fee_stroops` | `i128` | stroops (1 XLM = 10 000 000 stroops) | > 0 | `100000` (0.01 XLM) |
| `basic_sub_stroops` | `i128` | stroops | > 0 | `1000000` (0.1 XLM) |
| `pro_sub_stroops` | `i128` | stroops | > 0 | `3000000` (0.3 XLM) |
| `elite_sub_stroops` | `i128` | stroops | > 0 | `7000000` (0.7 XLM) |
| `sub_duration_secs` | `u64` | duration in seconds (not a Unix timestamp) | > 0 | `2592000` (30 days = 30 × 24 × 3600) |
| `pro_contact_limit` | `u32` | count | > 0 | `10` (10 contacts/period) |
| `trial_offer_escrow_stroops` | `i128` | stroops | > 0 | `500000` (0.05 XLM) |
| `trial_offer_expiry_secs` | `u64` | duration in seconds | > 0 | `3600` (1 hour) |

**Validation rules:**
- Every `i128` fee field must be > 0 (zero or negative → `InvalidInput` error code 15).
- `sub_duration_secs` must be > 0 (zero → `InvalidInput`).
- `pro_contact_limit` must be > 0 (zero → `InvalidInput`). This field caps the
  number of unique players a **Pro-tier** scout may contact within a single
  subscription period. Once the limit is reached, `pay_to_contact` returns
  `ProContactLimitReached` (code 20) for that scout until their subscription
  renews. **Elite-tier scouts are exempt** from this limit and may contact any
  number of players regardless of `pro_contact_limit`.

  > **Per-region overrides**: admins may configure a different limit for scouts
  > in specific regions using `set_regional_contact_limit(region, limit)`. When
  > set, the regional value takes precedence over this platform-wide default for
  > scouts whose registered `region` matches. See the
  > `set_regional_contact_limit` function reference below for details.
- `trial_offer_escrow_stroops` must be > 0 (zero or negative → `InvalidInput`). This is the XLM amount held in escrow when a scout logs a trial offer.
- `trial_offer_expiry_secs` must be > 0 (zero → `InvalidInput`). This defines the window within which a player must confirm a trial offer before it expires and the escrow is refunded.
- There is no enforced upper bound on fee fields, but values larger than the XLM supply
  (≈ 500 000 000 XLM = 5 × 10¹⁵ stroops) will cause `Overflow` errors at fee
  settlement time.

> [!NOTE]
> **ContactRecord vs ProContactPeriod — two-tracked quota**
> `ContactRecord` is a **permanent unlock**: created once per `(player_id, scout)` pair
> on successful `pay_to_contact` and never deleted. It gates duplicate-contact checks
> (`AlreadyContacted`).
>
> `ProContactPeriod` (stored under `ProContactCount`) is a **rolling quota counter**:
> it tracks how many *unique* players a **Pro-tier** scout has contacted in the *current
> subscription period* (`period_start == subscription.subscribed_at`). It resets to 0
> automatically when the scout renews/upgrades their subscription. Elite scouts bypass
> this counter entirely.
>
> **Interaction during `pay_to_contact`**: the contract first checks for an existing
> `ContactRecord` (permanent duplicate guard). If none exists and the scout is Pro
> tier, it then checks `ProContactPeriod.count < pro_contact_limit`. On success both
> are written — the permanent `ContactRecord` and the incremented `ProContactPeriod`.

See the [Glossary](GLOSSARY.md#feeconfig) for a plain-language description of each field.

### `EvidenceAccessGrant` Struct

On-chain proof that a scout is authorized to request the off-chain wrapped
decryption key for a player's confidential evidence. See
[EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md) for the full model, including why
this is an append-only fact rather than a live entitlement check.

| Field | Rust Type | Description |
|---|---|---|
| `player_id` | `u64` | Player whose evidence this grant authorizes access to. |
| `scout` | `Address` | Scout wallet authorized by this grant. |
| `granted_at` | `u64` | Unix-seconds ledger timestamp when the grant was issued. |
| `tier_at_grant` | `SubscriptionTier` | The scout's subscription tier at the moment of issuance. Recorded for audit only; not re-checked afterward. |
| `revoked` | `bool` | `true` once `admin_revoke_evidence_access` has been called for this grant. |
| `revoked_at` | `Option<u64>` | Unix-seconds ledger timestamp of revocation, if any. |

Written exactly once, atomically, by a successful `pay_to_contact` or
`batch_contact_players` call — never on a rejected call. Never deleted;
`admin_revoke_evidence_access` only flips `revoked`/`revoked_at`.

> [!NOTE]
> **Historical Fee Configs & Auditability**
> The `scout_access` contract stores the *current* `FeeConfig` on-chain (retrievable via `get_fee_config`) and a bounded on-chain trail of the **last 5 previous configs** (retrievable via `get_fee_config_history`). The history list is maintained oldest-first and is capped at 5 entries; when the cap is reached the oldest entry is evicted on the next `update_fee_config` call.
>
> This lightweight on-chain trail lets you read the immediately-previous fee configuration without depending on the off-chain indexer, making it suitable for quick audits or on-chain fee-change verification. For a *complete*, unbounded audit trail — including all historical fee rates for verifying that a contact fee or subscription payment matched the rate in effect at that time — replay the `fee_config_updated` event logs via the off-chain indexer's `fee_config_history` table (see [001_initial_schema.sql](migrations/001_initial_schema.sql#L135-L148)).

### Functions

### ScoutAccessError Codes

| Code | Error | Description |
|------|-------|-------------|
| 1 | `AlreadyInitialized` | Contract has already been initialized |
| 2 | `NotInitialized` | Contract has not been initialized yet |
| 3 | `ContractPaused` | Contract is paused by admin (circuit breaker) |
| 4 | `Unauthorized` | Caller is not authorized for this operation |
| 5 | `InsufficientFee` | Payment amount is below the required fee |
| 6 | `ScoutNotSubscribed` | Scout does not have an active subscription |
| 7 | `SubscriptionExpired` | Scout's subscription has expired |
| 8 | `AlreadyContacted` | Scout has already contacted this player |
| 9 | `InvalidTier` | Subscription tier value is not valid |
| 10 | `Overflow` | Arithmetic overflow in fee calculation |
| 11 | `TrialOfferNotFound` | Trial offer record does not exist |
| 12 | `PlayerNotRegistered` | Player is not registered in the registration contract |
| 13 | `ScoutNotRegistered` | Scout is not registered in the registration contract |
| 14 | `PlayerCapReached` | Maximum number of players per scout has been reached |
| 15 | `SubscriptionNotFound` | Subscription record not found for this scout |
| 16 | `ContactRecordNotFound` | Contact record not found |
| 17 | `TrialOfferExpired` | Trial offer has passed its expiry ledger |
| 18 | `InvalidSubscriptionDuration` | Subscription duration value is not valid |
| 19 | `FeeConfigNotFound` | Fee configuration has not been set |
| 20 | `TokenTransferFailed` | XLM or platform token transfer failed |
| 21 | `InvalidContactFee` | Contact fee value is not valid |
| 22 | `InvalidSubFee` | Subscription fee value is not valid |
| 23 | `EliteOnlyFeature` | Operation requires an Elite-tier subscription |
| 24 | `MigrationAlreadyComplete` | Migration has already been completed |
| 25 | `MigrationNotFound` | Migration record does not exist |
| 26 | `InvalidMigrationVersion` | Migration version number is not valid |
| 27 | `MigrationDataCorrupted` | Migration data failed integrity check |
| 28 | `MigrationStateMismatch` | Migration state does not match expected state |
| 29 | `MigrationNotActive` | Migration is not currently active |
| 30 | `MigrationReplayDetected` | Migration replay attempt detected |
| 31 | `MigrationConflict` | Migration conflicts with existing state |
| 32 | `MigrationVersionMismatch` | Migration version does not match current contract version |
| 33 | `MigrationChecksumFailed` | Migration checksum verification failed |
| 34 | `MigrationRollbackFailed` | Migration rollback could not be completed |
| 35 | `SubscriptionRecordEvicted` | Subscription record was evicted from contract storage |
| 36 | `PayToContactPaused` | Pay-to-contact feature is currently paused |
| 37 | `TrialEscrowNotOutstanding` | No outstanding trial escrow exists for this player |

---

#### `initialize(admin: Address, xlm_token: Address, fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

One-time contract setup. Validates that `xlm_token` points at a deployed
token contract by invoking `decimals()` on it, and that all fee fields
are positive with `sub_duration_secs` non-zero. The token probe is
read-only and side-effect-free; it exists so that a wrong `xlm_token`
address (testnet SAC on mainnet, a typo, a plain account, or a
non-token contract) is rejected immediately at deploy time rather than
surfacing as an opaque failure on the first `subscribe()` call.

| | |
|---|---|
| **Auth** | `admin` must sign |
| **Errors** | `AlreadyInitialized` · `InvalidInput` (zero or negative fee field, or `xlm_token` is not a callable token contract) |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- initialize \
  --admin $ADMIN_ADDRESS \
  --xlm_token $XLM_TOKEN_ADDRESS \
  --fee_config '{"contact_fee_stroops":100000,"basic_sub_stroops":1000000,"pro_sub_stroops":3000000,"elite_sub_stroops":7000000,"sub_duration_secs":2592000,"pro_contact_limit":10,"trial_offer_escrow_stroops":500000,"trial_offer_expiry_secs":3600}'
```

---

#### `propose_admin(new_admin: Address) -> Result<(), ScoutAccessError>`

Store or replace a pending admin proposal. The current admin retains all
privileges until the proposed address accepts.

| | |
|---|---|
| **Auth** | Current admin must sign |
| **Errors** | `NotInitialized` |
| **Emits** | `admin_transfer_proposed` with `(old_admin, new_admin)` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- propose_admin --new_admin $NEW_ADMIN_ADDRESS
```

---

#### `accept_admin() -> Result<(), ScoutAccessError>`

Finalize the transfer. The stored pending admin must sign, proving control of
the address. Acceptance updates the admin and clears the proposal.

| | |
|---|---|
| **Auth** | Pending admin must sign |
| **Errors** | `NotInitialized` · `PendingAdminNotSet` |
| **Emits** | `admin_transferred` with `(old_admin, new_admin)` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- accept_admin
```

---

#### `transfer_admin(new_admin: Address) -> Result<(), ScoutAccessError>`

Deprecated compatibility alias for `propose_admin`. It creates or replaces a
proposal and does not immediately change the admin.

---

#### `set_progress_contract(addr: Address) -> Result<(), ScoutAccessError>`

Register the progress contract address so `log_trial_offer` can call
`advance_level` cross-contract (admin only). Unlike
`verification.set_progress_contract`, this has no first-call-only guard —
it can always be re-invoked to re-wire the link. Bumps the link's re-wiring
epoch and emits `wiring_updated` (`link = "progress_contract"`) in addition
to `progress_contract_updated` (issue #1041).

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- set_progress_contract --addr $PROGRESS_CONTRACT_ID
```

---

#### `update_progress_contract(addr: Address) -> Result<(), ScoutAccessError>`

Alias for `set_progress_contract`, provided for naming consistency with
`verification.update_progress_contract` so the same verb can be used to
re-wire the progress contract link across contracts.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- update_progress_contract --addr $NEW_PROGRESS_CONTRACT_ID
```

---

#### `get_progress_contract() -> Option<Address>`

Returns the configured progress contract address, or `None` when the link has
not yet been configured. Read-only and requires no auth.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

---

#### `set_registration_contract(addr: Address) -> Result<(), ScoutAccessError>`

Wire the registration contract address for Pro-tier scout verification
gating (admin only). No first-call-only guard — freely re-settable. Bumps
the link's re-wiring epoch and emits `wiring_updated`
(`link = "registration_contract"`) in addition to
`registration_contract_updated` (issue #1041).

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- set_registration_contract --addr $REGISTRATION_CONTRACT_ID
```

---

#### `get_wiring_state() -> ScoutAccessWiringState`

Returns a snapshot of both peer-address pointers this contract holds
(`progress_contract`, `registration_contract`), each as a
`WiringLink { address: Option<Address>, epoch: u32 }`. `is_fully_wired()`
returns `true` iff both links are configured. Read-only, no auth required —
see [`docs/WIRING_REGISTRY_DESIGN.md`](WIRING_REGISTRY_DESIGN.md).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_wiring_state
```

---

#### `update_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

Adjust subscription and contact fee rates. Same validation rules as
`initialize`. This is an atomic, immediate, no-delay path that coexists by
design with the timelocked `propose_fee_config` / `activate_fee_config`
mechanism (see [`docs/FEE_CONFIG_PROPOSAL_DESIGN.md`](FEE_CONFIG_PROPOSAL_DESIGN.md#option-coexist-chosen)) —
it deliberately bypasses the latter's 7-day advance-notice guarantee and
remains callable by the admin at any time, for any fee change (increase or
decrease).

> [!NOTE]
> **Historical Fee Configs & Auditability**
> Adjusting the fee config emits the `fee_config_updated` event containing both the old and new `FeeConfig` values and also pushes the previous config into the bounded on-chain history (last 5 entries, oldest-first, accessible via `get_fee_config_history`). For a complete unbounded audit trail, replay events into the indexer's `fee_config_history` table (see [001_initial_schema.sql](migrations/001_initial_schema.sql#L135-L148)).
>
> **This call also emits a second, additive event: `fee_config_delay_bypassed`** — same `(old_config, new_config)` data shape as `fee_config_updated`, emitted in the same transaction — specifically so that indexers/auditors can distinguish "this fee change bypassed the 7-day delay via `update_fee_config`" from "this fee change went through `activate_fee_config` after the full delay," which otherwise emit an identical `fee_config_updated` event and are not otherwise distinguishable from the event stream alone. See [`docs/FEE_CONFIG_PROPOSAL_DESIGN.md`](FEE_CONFIG_PROPOSAL_DESIGN.md#fee_config_delay_bypassed-new-1055).

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InvalidInput` |
| **Emits** | `fee_config_updated` with `(admin, old_config, new_config)`, immediately followed by `fee_config_delay_bypassed` with the same data |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- update_fee_config \
  --fee_config '{"contact_fee_stroops":200000,"basic_sub_stroops":2000000,"pro_sub_stroops":5000000,"elite_sub_stroops":10000000,"sub_duration_secs":2592000,"pro_contact_limit":20,"trial_offer_escrow_stroops":1000000,"trial_offer_expiry_secs":7200}'
```

---

#### `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect, giving scouts on-chain-enforced advance notice of any increase.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InvalidInput` · `PendingFeeConfigAlreadyExists` (another proposal already pending) |
| **Emits** | `fee_config_proposed` (always); may also emit `fee_config_updated` for decreases |

> [!NOTE]
> **Fee Increases vs Decreases**
> Fee *decreases* (all fees ≤ current) are immediately activated in the same transaction, with both `fee_config_proposed` and `fee_config_updated` events emitted.
> Fee *increases* (at least one fee > current) are stored as pending and require a 7-day activation delay, emitting only `fee_config_proposed`.
> This design ensures scouts benefit immediately from decreases while having one full week to react to increases.

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- propose_fee_config \
  --fee_config '{"contact_fee_stroops":300000,"basic_sub_stroops":2000000,"pro_sub_stroops":6000000,"elite_sub_stroops":15000000,"sub_duration_secs":2592000,"pro_contact_limit":20}'
```

---

#### `activate_fee_config() -> Result<(), ScoutAccessError>`

Activate a pending fee configuration proposal after the 7-day delay has elapsed.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NoPendingFeeConfig` · `FeeConfigProposalNotReady` (delay not yet elapsed) |
| **Emits** | `fee_config_updated` with `(admin, old_config, new_config)` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- activate_fee_config
```

---

#### `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect, giving scouts on-chain-enforced advance notice of any increase.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InvalidInput` · `PendingFeeConfigAlreadyExists` (another proposal already pending) |
| **Emits** | `fee_config_proposed` (always); may also emit `fee_config_updated` for decreases |

> [!NOTE]
> **Fee Increases vs Decreases**
> Fee *decreases* (all fees ≤ current) are immediately activated in the same transaction, with both `fee_config_proposed` and `fee_config_updated` events emitted.
> Fee *increases* (at least one fee > current) are stored as pending and require a 7-day activation delay, emitting only `fee_config_proposed`.
> This design ensures scouts benefit immediately from decreases while having one full week to react to increases.

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- propose_fee_config \
  --fee_config '{"contact_fee_stroops":300000,"basic_sub_stroops":2000000,"pro_sub_stroops":6000000,"elite_sub_stroops":15000000,"sub_duration_secs":2592000,"pro_contact_limit":20}'
```

---

#### `activate_fee_config() -> Result<(), ScoutAccessError>`

Activate a pending fee configuration proposal after the 7-day delay has elapsed.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NoPendingFeeConfig` · `FeeConfigProposalNotReady` (delay not yet elapsed) |
| **Emits** | `fee_config_updated` with `(admin, old_config, new_config)` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- activate_fee_config
```

---

#### `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect, giving scouts on-chain-enforced advance notice of any increase.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InvalidInput` · `PendingFeeConfigAlreadyExists` (another proposal already pending) |
| **Emits** | `fee_config_proposed` (always); may also emit `fee_config_updated` for decreases |

> [!NOTE]
> **Fee Increases vs Decreases**
> Fee *decreases* (all fees ≤ current) are immediately activated in the same transaction, with both `fee_config_proposed` and `fee_config_updated` events emitted.
> Fee *increases* (at least one fee > current) are stored as pending and require a 7-day activation delay, emitting only `fee_config_proposed`.
> This design ensures scouts benefit immediately from decreases while having one full week to react to increases.

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- propose_fee_config \
  --fee_config '{"contact_fee_stroops":300000,"basic_sub_stroops":2000000,"pro_sub_stroops":6000000,"elite_sub_stroops":15000000,"sub_duration_secs":2592000,"pro_contact_limit":20}'
```

---

#### `activate_fee_config() -> Result<(), ScoutAccessError>`

Activate a pending fee configuration proposal after the 7-day delay has elapsed.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NoPendingFeeConfig` · `FeeConfigProposalNotReady` (delay not yet elapsed) |
| **Emits** | `fee_config_updated` with `(admin, old_config, new_config)` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- activate_fee_config
```

---

#### `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect, giving scouts on-chain-enforced advance notice of any increase.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InvalidInput` · `PendingFeeConfigAlreadyExists` (another proposal already pending) |
| **Emits** | `fee_config_proposed` (always); may also emit `fee_config_updated` for decreases |

> [!NOTE]
> **Fee Increases vs Decreases**
> Fee *decreases* (all fees ≤ current) are immediately activated in the same transaction, with both `fee_config_proposed` and `fee_config_updated` events emitted.
> Fee *increases* (at least one fee > current) are stored as pending and require a 7-day activation delay, emitting only `fee_config_proposed`.
> This design ensures scouts benefit immediately from decreases while having one full week to react to increases.

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- propose_fee_config \
  --fee_config '{"contact_fee_stroops":300000,"basic_sub_stroops":2000000,"pro_sub_stroops":6000000,"elite_sub_stroops":15000000,"sub_duration_secs":2592000,"pro_contact_limit":20}'
```

---

#### `activate_fee_config() -> Result<(), ScoutAccessError>`

Activate a pending fee configuration proposal after the 7-day delay has elapsed.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NoPendingFeeConfig` · `FeeConfigProposalNotReady` (delay not yet elapsed) |
| **Emits** | `fee_config_updated` with `(admin, old_config, new_config)` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- activate_fee_config
```

---

#### `withdraw_fees(to: Address) -> Result<i128, ScoutAccessError>`

Transfer all accumulated platform fees to the given address. Returns the amount
withdrawn in stroops.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InsufficientFee` (zero balance) |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- withdraw_fees --to $TREASURY_ADDRESS
```

---

#### `refund_subscription(scout: Address, amount: i128) -> Result<(), ScoutAccessError>`

Emergency admin function to return `amount` XLM (stroops) from the contract
balance to a scout. Use when a scout is accidentally double-charged (e.g. by
the race condition the upgrade timing guard is designed to prevent).

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InvalidInput` (amount ≤ 0) |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- refund_subscription \
  --scout $SCOUT_ADDRESS \
  --amount 1000000
```

---

#### `set_regional_contact_limit(region: String, limit: u32) -> Result<(), ScoutAccessError>`

Set or update a per-region Pro-tier contact limit override.

When a Pro-tier scout calls `pay_to_contact` or `batch_contact_players`, the
quota check first looks for a regional override keyed by the scout's registered
`region` (from the registration contract). If an override exists it is used
**instead of** `FeeConfig.pro_contact_limit`. If no override exists the
platform-wide default applies (backward-compatible fallback).

Override storage is bounded (one `u32` per region string) and admin-managed
only — scouts cannot alter their own quota.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InvalidInput` (limit = 0) |
| **Emits** | `regional_contact_limit_set` with `(admin, region, limit)` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- set_regional_contact_limit \
  --region "North America" \
  --limit 20
```

---

#### `remove_regional_contact_limit(region: String) -> Result<(), ScoutAccessError>`

Remove a previously-set per-region Pro-tier contact limit override.

After removal, scouts in that region fall back to the platform-wide
`FeeConfig.pro_contact_limit`. No-ops silently if no override existed for the
given region.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- remove_regional_contact_limit \
  --region "North America"
```

---

#### `subscribe(scout: Address, tier: SubscriptionTier) -> Result<(), ScoutAccessError>`

Purchase a `Basic`, `Pro`, or `Elite` subscription. The XLM fee is transferred
from the scout's wallet to the contract atomically. Downgrades to a cheaper
tier while a subscription is still active are rejected.

> **No-Proration Policy**: Upgrades to a higher tier do **not** provide credit
> for unused time on the previous subscription. The full new-tier fee is charged
> and `expires_at` is reset to `now + sub_duration_secs`. A minimum interval of
> 1 hour between `subscribe` calls from the same scout is enforced to prevent
> race conditions and double-charging.

| | |
|---|---|
| **Auth** | `scout` must sign and pre-approve the XLM transfer |
| **Errors** | `ContractPaused` · `NotInitialized` · `SubscriptionDowngradeNotAllowed` · `UpgradeTooSoon` · `Overflow` |

**Check precedence order** (when multiple error conditions are simultaneously
true, the first matching check in this list wins):

| Priority | Condition checked | Error returned |
|----------|-------------------|---------------|
| 1 | Contract is paused | `ContractPaused` (3) |
| 2 | Contract is not initialized | `NotInitialized` (2) |
| 3 | Scout auth | panic / host auth error |
| 4 | Active subscription exists AND requested tier rank < current tier rank | `SubscriptionDowngradeNotAllowed` (12) |
| 5 | Active subscription exists AND `now < subscribed_at + 3600 s` | `UpgradeTooSoon` (17) |
| 6 | Fee accumulation arithmetic overflows | `Overflow` (10) |
| 7 | `expires_at` calculation overflows | `Overflow` (10) |

> **Design note**: Checks 4 and 5 share the same outer `if` block — only one
> can fire per call. A downgrade attempt is evaluated before the timing guard,
> so a simultaneous downgrade-too-soon scenario returns `SubscriptionDowngradeNotAllowed`.

**Downgrade guard edge cases** (see issue #245 and tests in `scout_access/src/lib.rs`):

| Scenario | Behaviour |
|----------|-----------|
| First-time subscriber (no prior subscription record) | Guard is never reached; any tier may be chosen freely |
| Same-tier re-subscribe while active, after ≥ 1-hour interval | Allowed — `tier_rank(X) < tier_rank(X)` is false, so not a downgrade. `UpgradeTooSoon` still applies within the first hour |
| Same-tier re-subscribe within the first hour | Blocked by `UpgradeTooSoon` (17) — the guard's interval applies to same-tier renewals in addition to upgrades |
| Re-subscribe at exactly `expires_at` timestamp | **Blocked** — the condition is `now <= expires_at`, so the subscription is considered active through its final second. Wait for `now > expires_at` |
| Re-subscribe one second after `expires_at` | Allowed — subscription is expired; any lower tier is permitted |
| Pro (rank 2) → Basic (rank 1) while active | Blocked — `tier_rank(Basic)=1 < tier_rank(Pro)=2` triggers `SubscriptionDowngradeNotAllowed` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- subscribe \
  --scout $SCOUT_ADDRESS \
  --tier '"Elite"'
```

---

#### `set_auto_renew(scout: Address, enabled: bool) -> Result<(), ScoutAccessError>`

Opt a scout wallet in (`true`) or out (`false`) of automatic subscription renewal.

Once enabled, a keeper (off-chain cron job or bot) can call `renew_if_due` when
the scout's subscription is approaching expiry. The flag is stored in persistent
storage and survives upgrades.

| | |
|---|---|
| **Auth** | `scout` must sign |
| **Errors** | `ContractPaused` · `NotInitialized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- set_auto_renew \
  --scout $SCOUT_ADDRESS \
  --enabled true
```

---

#### `get_auto_renew(scout: Address) -> bool`

Returns `true` if the scout has opted in to automatic subscription renewal,
`false` otherwise (including for scouts who have never called `set_auto_renew`).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_auto_renew \
  --scout $SCOUT_ADDRESS
```

---

#### `renew_if_due(scout: Address) -> Result<(), ScoutAccessError>`

Renew a scout's subscription if auto-renewal is enabled and the subscription is
at or near expiry.

**Renewal window**: fires when the current timestamp is within the last 10 % of
`sub_duration_secs` before `expires_at`, **or** after `expires_at` has already
passed. Outside this window the function is a no-op and returns `Ok(())` without
charging — safe to call on a schedule.

**Auth model**: Soroban's `token::Client::transfer` always requires the sender's
authorization *in the same transaction*. A third-party keeper bot cannot pull XLM
from the scout's wallet on its own; the scout must sign the `renew_if_due`
transaction, just as they sign `subscribe`. The keeper's role is to remind the
scout to sign before expiry, not to charge them autonomously. A future
allowance-based (`token::approve`) pattern could enable truly permissionless
renewal, but is not implemented in this version.

| | |
|---|---|
| **Auth** | `scout` must sign |
| **Errors** | `ContractPaused` · `NotInitialized` · `AutoRenewNotEnabled` · `ScoutNotSubscribed` · `Overflow` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- renew_if_due \
  --scout $SCOUT_ADDRESS
```

---

#### `pay_to_contact(scout: Address, player_id: u64) -> Result<(), ScoutAccessError>`

Pay a micro-fee to unlock a player's contact details. Scout must have an active
(non-expired) subscription.

**Evidence access grant**: on success, atomically writes an
`EvidenceAccessGrant(player_id, scout)` and emits `evidence_access_granted` —
see [EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md). This never happens on a
rejected call.

**Pro-tier contact limit**: Pro-tier scouts are capped at `pro_contact_limit`
unique player contacts per subscription period (configured in `FeeConfig`).
Once the limit is reached, further `pay_to_contact` calls return
`ProContactLimitReached` (code 20) until the subscription renews. Elite-tier
scouts are **exempt** from this limit.

| | |
|---|---|
| **Auth** | `scout` must sign |
| **Errors** | `ContractPaused` · `NotInitialized` · `PayToContactPaused` · `ScoutNotSubscribed` · `SubscriptionExpired` · `AlreadyContacted` · `ProContactLimitReached` · `Overflow` |

**Check precedence order** (when multiple error conditions are simultaneously
true, the first matching check in this list wins):

| Priority | Condition checked | Error returned |
|----------|-------------------|---------------|
| 1 | Contract is paused | `ContractPaused` (3) |
| 2 | Contract is not initialized | `NotInitialized` (2) |
| 3 | `pay_to_contact` is paused (function-scoped, issue #1056) | `PayToContactPaused` (30) |
| 4 | Scout auth | panic / host auth error |
| 5 | No `Subscription` record exists for the scout | `ScoutNotSubscribed` (6) |
| 6 | `Subscription` record exists but `expires_at < now` | `SubscriptionExpired` (7) |
| 7 | `ContactRecord` already exists for `(player_id, scout)` | `AlreadyContacted` (8) |
| 8 | Scout is Pro tier AND `current_count >= pro_contact_limit` | `ProContactLimitReached` (20) |
| 9 | Fee accumulation arithmetic overflows | `Overflow` (10) |

> **Design note — paused vs unsubscribed (Priority 1 vs 5)**: when the
> contract is paused *and* the scout has no subscription, the caller sees
> `ContractPaused`, not `ScoutNotSubscribed`. A frontend can safely treat
> `ContractPaused` as "service unavailable, try again later" without
> needing to check subscription state. This ordering is intentional and
> consistent with every other state-changing function in this contract.

> **Design note — function-scoped vs whole-contract pause (Priority 3 vs 1)**:
> `pause_pay_to_contact` halts only `pay_to_contact`, while the whole-contract
> pause (Priority 1) still takes precedence. When only the function-scoped
> pause is active, scouts can still `subscribe` / renew / read state; only
> fee-charging contact is blocked. This mirrors `verification`'s
> `pause_approve_milestone` pattern (issue #809).

> **Design note — expired vs already-contacted (Priority 6 vs 7)**: an
> expired subscription takes precedence over a duplicate-contact guard.
> This is the more actionable error for the user ("renew your subscription")
> and prevents leaking whether a contact record exists to an unsubscribed
> caller.

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- pay_to_contact \
  --scout $SCOUT_ADDRESS \
  --player_id 1
```

---

#### `batch_contact_players(scout: Address, player_ids: Vec<u64>) -> Result<u32, ScoutAccessError>`

Contact multiple players in a single transaction. The contact fee is charged
once per new player; already-contacted players are silently skipped (no charge).
The total fee for all new contacts is deducted in a single token transfer.
Returns the count of new contacts recorded.

Scout must have an active (non-expired) subscription.

**Evidence access grant**: for each newly-recorded contact in the batch
(never for an already-contacted player that was silently skipped), atomically
writes an `EvidenceAccessGrant` and emits `evidence_access_granted`, same as
`pay_to_contact` — see [EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md).

| | |
|---|---|
| **Auth** | `scout` must sign |
| **Errors** | `ContractPaused` · `NotInitialized` · `ScoutNotSubscribed` · `SubscriptionExpired` · `ContactQuotaExceeded` · `Overflow` |

**Check precedence order** (when multiple error conditions are simultaneously
true, the first matching check in this list wins):

| Priority | Condition checked | Error returned |
|----------|-------------------|---------------|
| 1 | Contract is paused | `ContractPaused` (3) |
| 2 | Contract is not initialized | `NotInitialized` (2) |
| 3 | Scout auth | panic / host auth error |
| 4 | No active subscription (no record or expired) | `ScoutNotSubscribed` (6) or `SubscriptionExpired` (7) |
| 5 | Pro-tier contact quota would be exceeded by the batch | `ContactQuotaExceeded` (18) |
| 6 | `total_fee` multiplication overflows | `Overflow` (10) |

> **Design note — quota check before payment (Priority 5 before fee transfer)**:
> the quota check runs before the XLM transfer. This means no partial charge
> occurs when a batch would exceed the Pro monthly limit — the call fails cleanly
> and the scout can retry with a smaller batch.

> **Design note — `ContactQuotaExceeded` vs `ProContactLimitReached`**: this
> function uses `ContactQuotaExceeded` (18) via the `check_pro_contact_quota_with_count`
> helper, while `pay_to_contact` uses `ProContactLimitReached` (20) via a
> separate inline check. They enforce the same limit but return different error
> codes depending on the call path. Callers should handle both.

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- batch_contact_players \
  --scout $SCOUT_ADDRESS \
  --player_ids '[1,2,3]'
```

---

#### `log_trial_offer(scout: Address, player_id: u64, details_hash: String) -> Result<u32, ScoutAccessError>`

Record a trial offer on-chain and escrow `trial_offer_escrow_stroops` (from
`FeeConfig`) out of the scout's wallet. Scout must hold an active Elite
subscription. `details_hash` is an IPFS/Arweave CID of the offer document.

This is **step 1 of the two-step trial-offer flow** — it stores a
`TrialOffer` record and a `TrialEscrow(amount, expires_at)` record and emits
`trial_offer_logged`, but it does **not** call `progress.advance_level` and
does **not** advance the player's level. The player must call
[`confirm_trial_offer`](#confirm_trial_offerplayer_wallet-address-player_id-u64-index-u32-idempotency_nonce-optionstring---result-scoutaccesserror)
before `expires_at` to release the escrow and advance to Level 3; see that
entry for the confirmation-side checks and the expiry/refund branch. Returns
the trial offer index.

| | |
|---|---|
| **Auth** | `scout` must sign (Elite subscription required) |
| **Errors** | `ContractPaused` · `NotInitialized` · `InvalidInput` · `ScoutNotSubscribed` · `SubscriptionExpired` · `Unauthorized` · `TrialOfferRateLimited` · `Overflow` |

**Check precedence order** (when multiple error conditions are simultaneously
true, the first matching check in this list wins):

| Priority | Condition checked | Error returned |
|----------|-------------------|---------------|
| 1 | Contract is paused | `ContractPaused` (3) |
| 2 | Contract is not initialized | `NotInitialized` (2) |
| 3 | Scout auth | panic / host auth error |
| 4 | `details_hash` fails CID validation | `InvalidInput` (15) |
| 5 | No active subscription (no record or expired) | `ScoutNotSubscribed` (6) or `SubscriptionExpired` (7) |
| 6 | Subscription tier is not Elite | `Unauthorized` (4) |
| 7 | No `ContactRecord` exists for `(player_id, scout)` | `Unauthorized` (4) |
| 8 | Rate limit: within 24 h cooldown for `(scout, player_id)` | `TrialOfferRateLimited` (19) |
| 9 | Trial counter increment overflows | `Overflow` (10) |

> ✅ **Design note — `require_initialized` check added**: `log_trial_offer`
> now calls `require_initialized` immediately after `require_not_paused`,
> matching `subscribe`, `pay_to_contact`, and `batch_contact_players`.
> Fixed by the full guard-ordering audit (PR feat/797-798-801-835). See
> [Design Discussion §1](#1-log_trial_offer-is-missing-require_initialized--resolved).

> **Design note — `InvalidInput` before subscription check (Priority 4 before 5)**:
> `details_hash` is validated before the subscription is looked up. This means
> a scout with an expired subscription who also supplies a malformed CID sees
> `InvalidInput`, not `SubscriptionExpired`. Prefer validating inputs as early
> as possible; this ordering is correct.

> **Design note — both `Unauthorized` codes share priority 6 and 7**: the
> tier check and the previous-contact check both return `Unauthorized` (4)
> but are separate runtime conditions. If a caller has a non-Elite subscription
> *and* has never contacted the player, they will only ever see `Unauthorized`
> from the tier check (priority 6 fires first).

> **Design note — `TrialOfferRateLimited` vs `Unauthorized` ordering
> (Priority 8 after 6–7)**: the rate-limit check occurs after authorization.
> A non-Elite scout cannot trigger `TrialOfferRateLimited`; they will always
> see `Unauthorized` first.

> **Design note — `log_trial_offer` does not call `advance_level`**: level
> advancement and its `ProgressCallFailed` (14) error live entirely in
> `confirm_trial_offer`. A previous version of this document (and of the
> contract) had `log_trial_offer` advance the level directly; the
> escrow-based two-step flow replaced that in the current shipped contract
> and this doc now matches it.

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- log_trial_offer \
  --scout $SCOUT_ADDRESS \
  --player_id 1 \
  --details_hash '"QmTrialOfferDetails"'
```

---

#### `confirm_trial_offer(player_wallet: Address, player_id: u64, index: u32, idempotency_nonce: Option<String>) -> Result<(), ScoutAccessError>`

Confirm a previously logged trial offer. Called by the **player**, not the
scout — this is step 2 of the two-step trial-offer flow. On success, removes
the `TrialEscrow` record and calls `progress.advance_level` to advance the
player to Level 3 (Elite Tier).

If called after the escrow's `expires_at`, no level advancement is
attempted: the escrowed fee is refunded to the originating scout, the
`TrialEscrow` record is removed, `trial_offer_expired` is emitted, and the
call returns `Ok(())`. Returning an error would roll back the refund and
cleanup under Soroban transaction semantics. The scout must call
`log_trial_offer` again to create a new offer/escrow.

`idempotency_nonce` is optional. If supplied and that nonce was already
recorded by a prior successful confirmation, the call returns `Ok(())`
without re-running escrow cleanup or the `advance_level` call — this makes
it safe for a client to retry after a `ProgressCallFailed` error without
double-spending the escrow or re-advancing the level.

| | |
|---|---|
| **Auth** | `player_wallet` must sign |
| **Errors** | `ContractPaused` · `NotInitialized` · `TrialOfferAlreadyConfirmed` · `TrialOfferNotFound` · `InvalidInput` · `ProgressCallFailed` |

**Check precedence order** (when multiple error conditions are simultaneously
true, the first matching check in this list wins):

| Priority | Condition checked | Error returned |
|----------|-------------------|---------------|
| 1 | Contract is paused | `ContractPaused` (3) |
| 2 | Contract is not initialized | `NotInitialized` (2) |
| 3 | Player auth (`player_wallet`) | panic / host auth error |
| 4 | `idempotency_nonce` supplied and already recorded from a prior confirmation | *(none — returns `Ok(())`)* |
| 5 | No `TrialEscrow` record exists for `(player_id, index)` — never created, or already consumed by a prior confirmation/expiry sweep | `TrialOfferAlreadyConfirmed` (22) |
| 6 | No `TrialOffer` record exists for `(player_id, index)` | `TrialOfferNotFound` (11) |
| 7 | `now > escrow.expires_at` — escrow is refunded, the `TrialEscrow` record removed, and `trial_offer_expired` emitted | `Ok(())` |
| 8 | Progress contract not registered | `InvalidInput` (15) |
| 9 | Cross-contract `advance_level` fails | `ProgressCallFailed` (14) |

> **Design note — `TrialOfferAlreadyConfirmed` also covers "never existed"
> and "already expired" (Priority 5)**: the `TrialEscrow` record is removed
> both by a successful confirmation and by the expiry-refund branch (and by
> `expire_trial_offers`), so a missing escrow is ambiguous between "already
> confirmed," "already expired and refunded," and "no such offer was ever
> logged at this index." All three return `TrialOfferAlreadyConfirmed`;
> callers should treat it as "nothing left to confirm" rather than
> distinguishing the sub-cases.

> **Design note — idempotency short-circuit runs before the escrow load
> (Priority 4)**: a nonce is only recorded by a *successful* confirmation
> (it is persisted after `advance_level` succeeds, in the same transaction
> that consumes the escrow), so a recorded nonce implies the escrow is
> already gone. Checking the nonce first lets a client safely retry a
> timed-out confirmation and receive `Ok(())` instead of a misleading
> `TrialOfferAlreadyConfirmed`. There is no scenario where a recorded nonce
> coexists with an unexpired escrow, so the expiry check (Priority 7) can
> never be shadowed by this short-circuit.

> **Design note — escrow release is gated on the cross-contract call
> (Priority 9)**: the `TrialEscrow` record and its `OutstandingTrialEscrows`
> index entry are only removed after `advance_level` returns successfully.
> If the cross-contract call fails, the escrow is left in place so the call
> can be retried (subject to `expires_at`), rather than being silently
> forfeited.

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  --source $PLAYER_ADDRESS --network testnet \
  -- confirm_trial_offer \
  --player_wallet $PLAYER_ADDRESS \
  --player_id 1 \
  --index 1
```

---

#### `expire_trial_offers(limit: u32) -> Result<u32, ScoutAccessError>`

Admin-only sweep of pending trial offers whose escrow has passed
`expires_at`. For each expired entry it refunds the escrowed XLM to the
originating scout, removes the `TrialEscrow` record, and emits
`trial_offer_expired` — the same cleanup `confirm_trial_offer` performs
reactively when called late, run proactively and in bulk. Returns the
number of escrows actually swept (`0` if none were due).

`limit` bounds how many outstanding escrows are examined in this call,
capped server-side at 20 regardless of the value passed in, so a large
backlog cannot exceed the CPU-instruction budget in a single invocation
(see `ci/cpu-cost-budget.md`). Entries not yet past `expires_at` are left
in place. Call repeatedly (e.g. from a cron/keeper) to drain a backlog
larger than the per-call cap.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` · `Overflow` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- expire_trial_offers --limit 20
```

---

#### `has_contacted(scout: Address, player_id: u64) -> bool`

Return `true` if the scout has previously called `pay_to_contact` for this
player.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- has_contacted \
  --scout $SCOUT_ADDRESS \
  --player_id 1
```

---

#### `get_trial_count(player_id: u64) -> u32`

Return the total number of trial offers logged for a player.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_trial_count --player_id 1
```

---

#### `get_subscription(scout: Address) -> Result<Subscription, ScoutAccessError>`

Read a scout's current subscription record including tier and expiry timestamp.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `ScoutNotSubscribed` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_subscription --scout $SCOUT_ADDRESS
```

---

#### `get_fee_config() -> FeeConfig`

Return the current fee configuration.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- get_fee_config
```

---

#### `get_fee_config_history() -> Vec<FeeConfigHistoryEntry>`

Return the bounded on-chain history of the last (up to 5) `FeeConfig` values, **oldest-first**.

Each `FeeConfigHistoryEntry` contains:
- `config: FeeConfig` — the fee configuration that was active *before* a particular `update_fee_config` call.
- `updated_at: u64` — the Unix-seconds ledger timestamp when that change was made.

The *current* config is not included — retrieve it with `get_fee_config`. The history grows by
one entry per `update_fee_config` call and is capped at 5 entries; when the cap is reached the
oldest entry is evicted. This provides a lightweight middle-ground between the indexer-only
design (full history via `fee_config_updated` events) and an unbounded on-chain ring-buffer,
keeping the immediately-previous configs readable on-chain without additional indexer dependency.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- get_fee_config_history
```

---

#### `get_regional_contact_limit(region: String) -> u32`

Return the effective Pro-tier contact limit for the given `region`.

If a per-region override has been set via `set_regional_contact_limit`, that
value is returned. Otherwise the platform-wide `FeeConfig.pro_contact_limit`
is returned as the fallback. This is the same value the quota check inside
`pay_to_contact` and `batch_contact_players` uses for scouts registered in
that region.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_regional_contact_limit \
  --region "North America"
```

---

#### `get_accumulated_fees() -> i128`

Return total platform fees pending admin withdrawal (in stroops).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- get_accumulated_fees
```

---

#### `get_trial_offer(player_id: u64, index: u32) -> Result<TrialOffer, ScoutAccessError>`

Read a specific trial offer. Indices start at `1`.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `TrialOfferNotFound` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_trial_offer --player_id 1 --index 1
```

---

#### `pause_contract() -> Result<(), ScoutAccessError>`

Halt all state-changing operations.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- pause_contract
```

---

#### `unpause_contract() -> Result<(), ScoutAccessError>`

Resume normal operations after a pause.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- unpause_contract
```

---

#### `pause_pay_to_contact() -> Result<(), ScoutAccessError>`

Pause only the `pay_to_contact` function (function-scoped circuit breaker,
mirroring `verification.pause_approve_milestone` from issue #809; implemented
for scout_access in issue #1056).

This halts fee-charging contact while leaving every other function operational:
scouts can still `subscribe`, renew, read state, and use `batch_contact_players`
/ `log_trial_offer`. The whole-contract pause (if active) still takes precedence
over the function-scoped flag — un-pausing the whole contract does not clear
`pay_to_contact`'s flag. The flag is stored in instance storage under
`DataKey::PausedPayToContact` and defaults to `false`.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- pause_pay_to_contact
```

---

#### `unpause_pay_to_contact() -> Result<(), ScoutAccessError>`

Resume `pay_to_contact` after a function-scoped pause.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- unpause_pay_to_contact
```

---

#### `upgrade(new_wasm_hash: BytesN<32>) -> Result<(), ScoutAccessError>`

Upgrade the contract WASM. Admin auth required. Persistent storage survives.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- upgrade --new_wasm_hash $NEW_WASM_HASH
```

---

#### `get_scout_contacts(scout: Address) -> Vec<u64>`

Return the list of player IDs that a scout has unlocked via `pay_to_contact`
or `batch_contact_players`. This legacy method is unbounded; high-volume callers
should use `get_scout_contacts_page` instead.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_scout_contacts --scout $SCOUT_ADDRESS
```

---

#### `get_scout_contacts_page(scout: Address, offset: u32, limit: u32) -> ScoutContactsPage`

Return a bounded, paginated page of player IDs contacted by `scout`, together
with the total number of contacts.

This is the canonical paginated successor to the unbounded `get_scout_contacts`.
The `total` field lets callers determine when paging is complete without
over-fetching.

**Pagination**: `offset` is a zero-based item offset; `limit` is capped at 50 entries
per page, matching `get_global_milestone_index`. Returns an empty `entries` vec
when the offset is beyond the scout's contact list.

**Ordering**: entries are returned in contact order (oldest first).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_scout_contacts_page --scout $SCOUT_ADDRESS --offset 0 --limit 50
```

---

#### `get_all_trial_offers(player_id: u64) -> Vec<TrialOffer>`

Return all trial offers logged for a player in index order. Returns an
empty Vec if none exist.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_all_trial_offers --player_id 1
```

---

#### `health() -> ContractHealth`

Return the contract's initialization and pause status. `pay_to_contact_paused`
reflects the function-scoped pause (see `pause_pay_to_contact`); it is
independent of the whole-contract `paused` flag.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- health
```

---

#### `upgrade(new_wasm_hash: BytesN<32>) -> Result<(), ScoutAccessError>`

Replace the contract WASM in-place. Persistent storage (admin, subscriptions, trial offers) survives the upgrade. Admin only.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `NotInitialized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- upgrade --new_wasm_hash <NEW_WASM_HASH>
```

---

#### `get_scout_contacts(scout: Address) -> Vec<u64>`

Return all player IDs contacted by a scout as an O(1) index lookup (backed by `ScoutContacts` persistent storage key).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_scout_contacts --scout $SCOUT_ADDRESS
```

Direction check for the same contact relationship:

```bash
# Scout -> players contacted by this scout.
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_scout_contacts --scout $SCOUT_ADDRESS

# Player -> scouts that contacted this player.
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_player_contacts --player_id 1
```

---

#### `get_contact_record(scout: Address, player_id: u64) -> Option<ContactRecord>`

Retrieve the full `ContactRecord` for a `(player_id, scout)` pair. Returns
`None` if the scout has not contacted this player.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_contact_record --scout $SCOUT_ADDRESS --player_id 1
```

---

#### `get_player_contacts(player_id: u64) -> Vec<Address>`

Return all scout addresses that have contacted `player_id` as an O(1) index
lookup. Players can audit their inbound contact history directly from
on-chain state without replaying off-chain events.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_player_contacts --player_id 1
```

---

#### `restore_subscription_record(scout: Address) -> Result<(), ScoutAccessError>`

Re-extend the TTL of a `Subscription` record that is nearing archival so its
history remains available on-chain. Admin-only. Returns
`SubscriptionRecordEvicted` (code 29) if the entry has already been fully
evicted (key absent) and is unrecoverable.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` · `SubscriptionRecordEvicted` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- restore_subscription_record --scout $SCOUT_ADDRESS
```

---

#### `get_player_trial_offers(player_id: u64) -> Vec<TrialOffer>`

Return all trial offers for a given player in ascending index order (1..=N).
Returns an empty Vec for a player with no trial offers. Unbounded (unlike the
20-entry cap on `get_all_trial_offers`).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_player_trial_offers --player_id 1
```

---

#### `get_scout_trial_offers(scout: Address) -> Vec<(u64, u32)>`

Return all `(player_id, trial_index)` tuples for every trial offer logged by
`scout`, in insertion order (oldest first). Returns an empty Vec for a scout
who has not logged any trial offers. Each tuple can be passed to
`get_trial_offer(player_id, index)` to fetch the full offer record.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_scout_trial_offers --scout $SCOUT_ADDRESS
```

---

#### `get_all_trial_offers(player_id: u64) -> Vec<TrialOffer>`

Return all trial offers for a player in a single call. Bounded at 20 to prevent gas exhaustion. Returns an empty `Vec` when no offers exist.

| Function | Behavior | Recommended use |
|---|---|---|
| `get_all_trial_offers` | Returns at most 20 offers. | Bounded UI previews or low-cost reads. |
| `get_player_trial_offers` | Reads the complete per-player offer range. | Full history views or audits that must include entries beyond the first 20. |

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_all_trial_offers --player_id 1
```

---

#### `get_subscribers_by_tier(tier: SubscriptionTier) -> Vec<Address>`

Return all scout addresses currently subscribed at `tier` (an O(1) index
lookup backed by the `TierSubscribers` persistent storage key). Includes
expired subscriptions that have not yet been superseded by a renewal or
downgrade.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_subscribers_by_tier --tier '"Elite"'
```

---

#### `get_expiring_subscriptions(before_timestamp: u64, limit: u32) -> Vec<Subscription>`

Return subscriptions whose `expires_at` is at or before `before_timestamp`.
This query uses a day-granularity expiry bucket index to avoid scanning every
subscription, and it filters renewals by re-checking the live stored
`Subscription.expires_at`. The bucket scan starts at the earliest populated
bucket day (`DataKey::MinExpiryBucketDay`, tracked by `subscribe`/seeding), so
its cost tracks the number of populated expiry days in range rather than the
number of days elapsed since the epoch.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_expiring_subscriptions --before_timestamp 1700000000 --limit 50
```

---

#### `has_evidence_access(player_id: u64, scout: Address) -> bool`

Return `true` if `scout` currently holds a non-revoked `EvidenceAccessGrant`
for `player_id`. This is the fast check the off-chain key-wrapping service
should call before honoring a wrapped-key request. See
[EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- has_evidence_access --player_id 1 --scout $SCOUT_ADDRESS
```

---

#### `get_evidence_access_grant(player_id: u64, scout: Address) -> Option<EvidenceAccessGrant>`

Return the full grant record for `(player_id, scout)`, if one has ever been
issued — including a revoked grant, so callers can distinguish "never
granted" from "granted, then revoked".

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_evidence_access_grant --player_id 1 --scout $SCOUT_ADDRESS
```

---

#### `get_player_access_grants(player_id: u64, offset: u32, limit: u32) -> Vec<EvidenceAccessGrant>`

Page through every `EvidenceAccessGrant` ever issued for `player_id`,
oldest-first, for a player-facing "who has access to my evidence" audit UI.

`limit` is capped at 50 (`MAX_ACCESS_GRANT_PAGE_LIMIT`), matching the
on-chain index page size (`ACCESS_GRANT_PAGE_SIZE`), so a single call reads
at most two index pages plus one grant record per returned entry — cost
bounded by `limit`, independent of the player's total historical grant
count (proven at 1,000+ grants in `contracts/scout_access/tests/cost_budget.rs`).
Page through a player's full history by advancing `offset` by the number of
entries the previous call returned.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_player_access_grants --player_id 1 --offset 0 --limit 50
```

---

#### `admin_revoke_evidence_access(player_id: u64, scout: Address) -> Result<(), ScoutAccessError>`

Compliance/abuse takedown: mark an `EvidenceAccessGrant` revoked. Does not
delete the record — see [EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md) for why
grants are append-only facts, and why this only gates *future* key-wrap
requests rather than clawing back an already-delivered key. Idempotent:
revoking an already-revoked grant returns `Ok(())` without re-emitting the
event or changing `revoked_at`.

| | |
|---|---|
| **Auth** | admin must sign |
| **Errors** | `NotInitialized` · `GrantNotFound` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- admin_revoke_evidence_access --player_id 1 --scout $SCOUT_ADDRESS
```

---

Manages the trusted validator registry and milestone approvals.

| Function | Auth | Description |
|----------|------|-------------|
| `initialize(admin)` | admin | One-time setup, including default diversity configuration |
| `set_progress_contract(progress_contract)` | admin | Wire cross-contract link |
| `set_diversity_config(min_distinct_affiliations, gated_milestone_index)` | admin | Configure organizational diversity required for level advancement |
| `register_validator(wallet, credentials, affiliation)` | admin | Add a trusted validator with verified organization affiliation |
| `revoke_validator(wallet)` | admin | Deactivate validator |
| `approve_milestone(validator_wallet, player_id, description, evidence_hash)` | validator | Record milestone (with ledger_sequence for audit) + cross-call progress.advance_level |
| `get_milestone(player_id, index)` | — | Read a specific milestone |
| `get_milestone_count(player_id)` | — | Total milestones for a player |
| `get_diversity_config()` | — | Read affiliation diversity rules |
| `get_player_affiliation_count(player_id)` | — | Count distinct affiliated milestone approvers |
| `get_validator(wallet)` | — | Read validator record |
| `is_active_validator(wallet)` | — | Boolean check |
| `pause_contract()` / `unpause_contract()` | admin | Circuit breaker |
| `health()` | — | Returns true if initialized |

### Events

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `milestone_approved` | event_name, validator_address, milestone_index (u32) | player_id (u64), description (String), evidence_hash (String) | Emitted when a validator approves a player milestone with full milestone details |
| `validator_registered` | event_name | validator_address | Emitted when a new validator is registered |
| `validator_revoked` | event_name | validator_address | Emitted when a validator is deactivated |
| `milestone_disputed` | event_name, player_id, milestone_index | filer_address, jury_required | Emitted when a dispute is filed |
| `dispute_vote_cast` | event_name, player_id, milestone_index | validator_address, upheld | Emitted for each jury vote |
| `dispute_resolved` | event_name, player_id, milestone_index | upheld | Emitted after an admin resolution |
| `dispute_tallied` | event_name, player_id, milestone_index | upheld, votes_for, votes_against | Emitted after jury finalization |

---

## Shared Types

### `ProgressLevel`

Four-tier progress level used by all contracts. It is the core player ranking type
referenced throughout registration, verification, progress, and scout_access.

#### Variant table

| Ordinal | Variant | Semantic meaning |
|---------|---------|-----------------|
| 0 | `Unverified` | Profile created on-chain; no identity or performance verification has occurred yet. Default state for all newly registered players. |
| 1 | `VerifiedIdentity` | Identity confirmed by an approved academy or KYC validator. Player is discoverable by scouts with a Basic subscription or higher. |
| 2 | `PerformanceMilestones` | Performance statistics verified by an approved third-party validator. Player is discoverable by scouts with a Pro subscription or higher. |
| 3 | `EliteTier` | Scout feedback or a trial offer has been logged by an Elite-tier scout. Player is discoverable by scouts with an Elite subscription only. |

#### Subscription tier access mapping

| ProgressLevel | Minimum subscription tier to view |
|---------------|----------------------------------|
| `Unverified` (0) | None — public profile metadata only (no contact) |
| `VerifiedIdentity` (1) | Basic |
| `PerformanceMilestones` (2) | Pro |
| `EliteTier` (3) | Elite |

Scouts without a sufficient tier can still see that a player exists but cannot view
full profile details or initiate contact. Contact actions are separately gated by
`scout_access.contact_player`.

#### Valid transitions

Levels advance sequentially: 0 → 1 → 2 → 3. No skipping or reversing is permitted
except via the admin function `progress.reset_player_level`.

Level promotion is triggered by `verification.approve_milestone`, which cross-calls
[`progress.advance_level`](#advance_level-caller-address-player_id-u64-milestone_ref-u32---resultprogresslevel-progresserror).
The new level is also reflected in `registration` queries, including
[`registration.filter_players`](#filter_players-region-string-position-string-min_level-progresslevel---resultvecplayerprofile-scoutchainerror),
which accepts a `min_level` argument to restrict results to players at or above a
given tier.

### `ContractHealth`

```rust
pub struct ContractHealth {
    pub initialized: bool,
    pub paused: bool,
    /// Function-scoped pause flag for `pay_to_contact` (scout_access only).
    /// Always `false` for contracts that have no `pay_to_contact` function
    /// (`registration`, `verification`, `progress`).
    pub pay_to_contact_paused: bool,
}
```

### `PlayerVitals`

```rust
pub struct PlayerVitals {
    pub age: u32,
    pub position: String,  // max 64 bytes
    pub region: String,    // max 64 bytes
    pub nationality: String, // max 64 bytes
}
```

### `PlayerProfile`

```rust
pub struct PlayerProfile {
    pub player_id: u64,
    pub wallet: Address,
    pub vitals: PlayerVitals,
    pub ipfs_hashes: Vec<String>, // 1–10 entries
    pub level: ProgressLevel,
    pub registered_at: u64, // Unix seconds
    pub updated_at: u64,    // Unix seconds
}
```

### `ScoutProfile`

```rust
pub struct ScoutProfile {
    pub scout_id: u64,
    pub wallet: Address,
    pub region: String,   // max 128 bytes
    pub verified: bool,
    pub registered_at: u64, // Unix seconds
}
```

### `Validator`

```rust
pub struct Validator {
    pub wallet: Address,
    pub credentials: String,       // max 256 bytes
    pub registered_at: u64,        // Unix seconds
    pub active: bool,
    pub specializations: Vec<String>, // max 10 tags, each max 64 bytes
}
```

`specializations` is the list of milestone category tags this validator is
authorised to approve. An empty list means the validator is general-purpose
(can approve any untagged milestone). Tags are case-sensitive short strings
(e.g. `"physical-stats"`, `"identity-kyc"`, `"match-performance"`). Set at
registration time via `register_validator` or updated later via
`set_validator_specializations`.

### `ValidatorStatus`

```rust
pub enum ValidatorStatus {
    NotRegistered,
    Active,
    Revoked,
    RevokedForCause,
}
```

### `ValidatorActivityReport`

Convenience aggregate struct returned by `get_validator_activity_report`.
Bundles the fields from four individual queries into one response:

| Field | Source query | Description |
|---|---|---|
| `wallet` | — | Validator wallet address |
| `credentials` | `get_validator` | Human-readable credential label |
| `registered_at` | `get_validator` | Unix timestamp of registration |
| `active` | `get_validator` | Whether the validator is currently active |
| `status` | `get_validator_status` | Richer status (Active / Revoked / RevokedForCause / NotRegistered) |
| `milestone_count` | `get_validator_milestone_count` | Total milestones approved across all players |
| `distinct_player_count` | `get_validator_players` | Number of distinct players with at least one milestone |
| `distinct_players` | `get_validator_players` | List of distinct player IDs |

```rust
pub struct ValidatorActivityReport {
    pub wallet: Address,
    pub credentials: String,
    pub registered_at: u64,
    pub active: bool,
    pub status: ValidatorStatus,
    pub milestone_count: u32,
    pub distinct_player_count: u32,
    pub distinct_players: Vec<u64>,
}
```

### `Milestone`

```rust
pub struct Milestone {
    pub player_id: u64,
    pub validator: Address,
    pub description: String,
    pub evidence_hash: String,  // IPFS Qm… or Arweave bafy…, 2–128 bytes
    pub approved_at: u64,       // Unix seconds
    pub ledger_sequence: u32,   // Soroban ledger sequence number (not a timestamp)
}
```

### `PendingMilestoneClaim`

Bounded, fixed-size accumulator for a k-of-n `attest_milestone` claim, keyed
by `(player_id, evidence_hash)`. See `attest_milestone` above for the full
design rationale (claim identity, expiry, revoke-invalidation).

```rust
pub struct PendingMilestoneClaim {
    pub player_id: u64,
    pub evidence_hash: String,
    pub description: String,    // locked in by the first vote in this round
    pub vote_count: u32,        // distinct, currently-valid votes so far
    pub round: u32,             // bumped on voting-window expiry
    pub created_at: u64,        // Unix seconds this round started
    pub threshold: u32,         // snapshotted when this round started
}
```

### `AttestationStatus`

Return type of `attest_milestone`.

```rust
pub enum AttestationStatus {
    Pending(u32),    // vote recorded; new vote_count, still short of threshold
    Committed(u32),  // this vote reached threshold; payload is the milestone index
}
```

### `MilestoneDispute`

```rust
pub struct MilestoneDispute {
    pub player_id: u64,
    pub milestone_index: u32,
    pub reason: String,
    pub disputed_at: u64,       // Unix seconds
    pub resolved: bool,         // false until admin resolves the dispute
    pub upheld: bool,           // admin outcome; meaningful once resolved is true
}
```

### `ProgressEntry`

```rust
pub struct ProgressEntry {
    pub player_id: u64,
    pub old_level: ProgressLevel,
    pub new_level: ProgressLevel,
    pub updated_by: Address,
    pub updated_at: u64,        // Unix seconds
    pub milestone_ref: u32,     // links to verification contract index
    pub ledger_sequence: u32,   // Soroban ledger sequence number (not a timestamp)
}
```

### `HistoryProofStep`

One step of a `verify_history_proof` / `get_history_proof` Merkle inclusion
proof — see [Merkle history commitment](#merkle-history-commitment) above.

```rust
pub struct HistoryProofStep {
    pub sibling: BytesN<32>,
    pub sibling_is_right: bool, // combine as H(current, sibling) if true, else H(sibling, current)
}
```

### `SubscriptionTier`

```rust
pub enum SubscriptionTier {
    Basic,  // browse Level 1+ players
    Pro,    // browse all levels + up to 10 contacts/month
    Elite,  // unlimited contacts + trial offer logging
}
```

### `Subscription`

```rust
pub struct Subscription {
    pub scout: Address,
    pub tier: SubscriptionTier,
    pub expires_at: u64,        // Unix seconds
    pub subscribed_at: u64,     // Unix seconds
}
```

### `ContactRecord`

```rust
pub struct ContactRecord {
    pub player_id: u64,
    pub scout: Address,
    pub contacted_at: u64,      // Unix seconds
}
```

### `FeeConfig`

```rust
pub struct FeeConfig {
    pub contact_fee_stroops: i128,          // must be > 0
    pub basic_sub_stroops: i128,            // must be > 0
    pub pro_sub_stroops: i128,              // must be > 0
    pub elite_sub_stroops: i128,            // must be > 0
    pub sub_duration_secs: u64,             // duration in seconds, must be > 0 (not a Unix timestamp)
    pub pro_contact_limit: u32,             // must be > 0; Elite scouts bypass this cap
    pub trial_offer_escrow_stroops: i128,   // escrow held per trial offer, must be > 0
    pub trial_offer_expiry_secs: u64,       // confirmation window in seconds, must be > 0
}
```

> [!NOTE]
> **Historical Fee Configs & Auditability (Proposal + Activation Pattern)**
> The `scout_access` contract stores the *current* `FeeConfig` on-chain (retrievable via `get_fee_config`) and optionally a pending proposal (when an increase is being staged for activation).
> 
> Historical fee configurations must be reconstructed off-chain by replaying events into the indexer's `fee_config_history` table:
> - `fee_config_proposed` marks when an increase is staged (proposal timestamp, proposed config).
> - `fee_config_updated` marks when a config *takes effect* (either immediately for decreases, or after the 7-day delay for increases, or immediately via the `update_fee_config` bypass — see next bullet).
> - `fee_config_delay_bypassed` accompanies `fee_config_updated` in the same transaction *only* when `update_fee_config` was the source, letting the audit trail distinguish a delay-bypassing change from a delay-respecting `activate_fee_config` (#1055).
> 
> The audit trail is complete: every config change is visible via one of these events, subscribers can be notified of coming increases well in advance, and whether any given change honored that advance notice is itself auditable.


### `ProContactPeriod`

```rust
pub struct ProContactPeriod {
    pub period_start: u64,      // Unix seconds
    pub count: u32,
}
```

### `TrialOffer`

```rust
pub struct TrialOffer {
    pub player_id: u64,
    pub scout: Address,
    pub details_hash: String, // IPFS/Arweave CID
    pub logged_at: u64,         // Unix seconds
}
```

---

## Error Codes

### `ScoutChainError` (registration contract)

| Code | Variant | Common Cause |
|------|---------|--------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotInitialized` | Operation before `initialize` |
| 3 | `PlayerNotFound` | Invalid `player_id` |
| 4 | `ValidatorNotAuthorized` | Unregistered account approving milestone |
| 5 | `InvalidProgressTransition` | Skipping or reversing a level |
| 6 | `ScoutNotSubscribed` | Scout has no subscription |
| 7 | `InsufficientFee` | Underpaying contact fee |
| 8 | `AlreadyRegistered` | Wallet already has a profile for this role |
| 9 | `ContractPaused` | Circuit breaker is active |
| 10 | `Unauthorized` | Wrong account for a privileged operation |
| 11 | `Overflow` | Counter or fee arithmetic overflowed |
| 12 | `ScoutNotFound` | Invalid `scout_id` |
| 13 | `InvalidInput` | Field too long, bad hash count, or empty value |
| 14 | `PendingAdminNotSet` | `accept_admin` called without a pending proposal |
| 15 | `PlayerCapReached` | Player registration cap reached — a hard stop, not retryable |
| 16 | `RegistrationCooldown` | Caller registered again before the cooldown elapsed — retryable |
| 17 | `PlayerRecordEvicted` | `restore_player_record` targeted a fully evicted, unrecoverable player entry |
| 18 | `ScoutRecordEvicted` | `restore_scout_record` targeted a fully evicted, unrecoverable scout entry |

### `VerificationError` (verification contract)

| Code | Variant | Common Cause |
|------|---------|--------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotInitialized` | Operation before `initialize` |
| 3 | `ContractPaused` | Circuit breaker is active |
| 4 | `Unauthorized` | Wrong account for a privileged operation |
| 5 | `ValidatorNotFound` | Wallet not in validator registry |
| 6 | `ValidatorInactive` | Validator has been revoked |
| 7 | `ValidatorAlreadyRegistered` | Wallet already registered as validator |
| 8 | `PlayerNotFound` | Invalid `player_id` |
| 9 | `InvalidInput` | Bad evidence hash, credentials too long, or region too long |
| 10 | `ReasonTooLong` | Revocation reason exceeds 128 bytes |
| 11 | `AlreadyConfigured` | `set_progress_contract` called twice |
| 12 | `ProgressCallFailed` | Cross-contract `advance_level` failed |
| 13 | `Overflow` | Milestone counter overflowed |
| 14 | `MilestoneNotFound` | Index out of range |
| 15 | `ValidatorCapReached` | 100-validator limit reached; contract upgrade required to raise the cap |
| 16 | `DuplicateEvidence` | Evidence hash has already been used in a prior `approve_milestone` call |
| 17 | `MilestoneLimitExceeded` | Validator has already approved 5 milestones for this player |
| 18 | `DisputeAlreadyResolved` | Dispute was already resolved and cannot be resolved again |
| 19 | `PendingAdminNotSet` | `accept_admin` called before an admin transfer was proposed |
| 20 | `ApproveMilestonePaused` | `approve_milestone` is paused independently of the whole-contract pause |
| 21 | `SpecializationMismatch` | `milestone_category` supplied to `approve_milestone` but the validator is not tagged for that category |
| 22 | `InvalidAttestation` | ed25519 signature over the attestation payload failed, or its contract/network binding does not match this instance |
| 23 | `AttestationKeyNotFound` | No attestation public key has been registered for this validator |
| 24 | `InvalidNonce` | Attestation nonce is not strictly greater than the last accepted nonce |
| 25 | `RegistrationCooldown` | Validator registration attempted before the cooldown window elapsed |
| 26 | `DuplicateAttestation` | Same active validator attested to the same `(player_id, evidence_hash)` claim within its current voting round |
| 27 | `TooManyPendingVotes` | Validator already has `MAX_PENDING_VOTES_PER_VALIDATOR` concurrent open attestation votes |
| 28 | `ThresholdModeRequiresAttestation` | `approve_milestone` / `submit_attested_milestone` called while `get_milestone_threshold() > 1` — use `attest_milestone` |
| 29 | `RegistrationCallFailed` | Cross-contract call to the registration contract failed |
| 30 | `MigrationNotActive` | Migration window is not currently active; call `open_migration_window` first |
| 31 | `MilestoneAlreadyExists` | A `Milestone` already exists at `(player_id, milestone_index)` with different content |
| 32 | `DisputeAlreadyExists` | A `MilestoneDispute` already exists at `(player_id, milestone_index)` with different content |
| 33 | `ValidatorRecordEvicted` | `restore_validator_record` targeted a validator entry whose archival grace period has fully elapsed |
| 34 | `MilestoneRecordEvicted` | `restore_milestone_record` targeted a milestone entry that has been fully evicted |
| 35 | `NotEligibleToReReview` | `rereview_milestone` called by a wallet that is not a currently-active validator |
| 36 | `MilestoneNotFlagged` | `rereview_milestone` called on a milestone not currently flagged as pending re-review |
| 37 | `DisputeRequiresJury` | `resolve_dispute` called on a dispute that requires jury resolution — use `tally_dispute` |
| 38 | `NotJuryDispute` | `cast_dispute_vote` / `tally_dispute` called on a dispute not routed to the jury path |
| 39 | `VotingWindowClosed` | `cast_dispute_vote` called after the voting window has closed |
| 40 | `ConflictOfInterest` | `cast_dispute_vote` called by the validator who approved the disputed milestone |
| 41 | `AlreadyVoted` | `cast_dispute_vote` called by a validator who has already voted on this dispute |
| 42 | `VotingWindowOpen` | `tally_dispute` called before the window closes with votes tied at or above quorum |
| 43 | `QuorumNotReached` | `tally_dispute` called before the window closes and quorum not yet reached |

### `ProgressError` (progress contract)

| Code | Variant | Common Cause |
|------|---------|--------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotInitialized` | Operation before `initialize` |
| 3 | `ContractPaused` | Circuit breaker is active |
| 4 | `Unauthorized` | Wrong account for a privileged operation |
| 5 | `InvalidProgressTransition` | Level skip or reversal attempted |
| 6 | `AlreadyAtMaxLevel` | Player is already at `EliteTier` |
| 7 | `PlayerNotFound` | History index out of range |
| 8 | `Overflow` | History counter overflowed |
| 9 | `RegistrationCallFailed` | Cross-contract call to registration contract failed when syncing player level |
| 10 | `PendingAdminNotSet` | `accept_admin` called without a pending proposal |

### `ScoutAccessError` (scout_access contract)

| Code | Variant | Common Cause |
|------|---------|--------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotInitialized` | Operation before `initialize` |
| 3 | `ContractPaused` | Circuit breaker is active |
| 4 | `Unauthorized` | Wrong account or non-Elite tier for trial offer |
| 5 | `InsufficientFee` | Scout underpaid a subscription or contact fee |
| 6 | `ScoutNotSubscribed` | No subscription record found |
| 7 | `SubscriptionExpired` | Subscription past `expires_at` |
| 8 | `AlreadyContacted` | Duplicate `pay_to_contact` for same player |
| 9 | `InvalidTier` | Unknown subscription tier |
| 10 | `Overflow` | Fee accumulation arithmetic overflowed |
| 11 | `TrialOfferNotFound` | Index out of range |
| 12 | `SubscriptionDowngradeNotAllowed` | Downgrade attempted while subscription active |
| 14 | `ProgressCallFailed` | Cross-contract `advance_level` failed |
| 15 | `InvalidInput` | Zero or negative fee field in `FeeConfig` |
| 16 | `NoFeesToWithdraw` | No accumulated fees available to withdraw |
| 17 | `UpgradeTooSoon` | Subscribe called before minimum interval elapsed |
| 18 | `ContactQuotaExceeded` | **DEPRECATED** — slot reserved; callers should use `ProContactLimitReached` (20) for the Pro-tier monthly contact limit condition |
| 19 | `TrialOfferRateLimited` | Elite scout sent a trial offer to the same player within the cooldown window — the offer was already logged; retry after the cooldown expires |
| 20 | `ProContactLimitReached` | Pro-tier scout has reached the `pro_contact_limit` contacts for the current subscription period (Elite scouts are exempt from this limit) |
| 21 | `PendingAdminNotSet` | `accept_admin` called before an admin transfer was proposed via `propose_admin` |
| 22 | `TrialOfferAlreadyConfirmed` | `confirm_trial_offer` called twice for the same trial offer |
| 23 | `TrialOfferExpired` | Legacy compatibility code; expiry confirmation now commits the refund and returns success |
| 24 | `NoPendingFeeConfig` | `activate_fee_config` called with no pending proposal to activate |
| 25 | `FeeConfigProposalNotReady` | `activate_fee_config` called before the pending proposal's activation delay elapsed |
| 26 | `PendingFeeConfigAlreadyExists` | `propose_fee_config` called while a pending proposal already exists |
| 27 | `ScoutNotVerified` | Pro-tier `subscribe()` rejected an unverified (or not-found) scout — see [`docs/SYBIL_MITIGATION_DESIGN.md`](SYBIL_MITIGATION_DESIGN.md) |
| 28 | `AutoRenewNotEnabled` | `renew_if_due` called for a scout without auto-renewal enabled |
| 29 | `SubscriptionRecordEvicted` | `restore_subscription_record` targeted a subscription entry whose archival grace period has fully elapsed (evicted, not merely archived) and is unrecoverable |
| 30 | `PayToContactPaused` | `pay_to_contact` called while the function-scoped pause is active (issue #1056) — the whole-contract `ContractPaused` (3) takes precedence when both are set |

---

## Events

All events follow the unified `(Symbol, actor)` topic schema introduced in #246. Soroban event indexers can filter any event by actor address using the second topic element.

**Standard schema**: `topics: (event_name: Symbol, actor: Address)` · `data: (entity_id, ...other_fields)`

### registration

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `player_registered` | event_name, wallet (Address) | player_id (u64) | New player profile created |
| `scout_registered` | event_name, wallet (Address) | scout_id (u64) | New scout profile created |
| `profile_updated` | event_name, wallet (Address) | player_id (u64) | Player updates IPFS content hashes |
| `player_deregistered` | event_name, admin (Address) | player_id (u64) | Admin removes a player profile |
| `player_deactivated` | event_name, admin (Address) | player_id (u64) | Admin soft-hides a player from filter results |
| `player_reactivated` | event_name, admin (Address) | player_id (u64) | Admin restores a soft-hidden player to filter results |
| `scout_verified` | event_name, wallet (Address) | scout_id (u64) | Admin verifies a scout |
| `player_level_synced` | event_name, progress_contract (Address) | player_id (u64) | Progress contract syncs a player's level |
| `admin_transfer_proposed` | event_name, old_admin (Address) | new_admin (Address) | Current admin proposes a replacement |
| `admin_transferred` | event_name, old_admin (Address) | new_admin (Address) | Pending admin accepts control |
| `wiring_updated` | event_name, admin (Address), link (Symbol) | new_address (Address), new_epoch (u32) | `set_progress_contract` re-wired the `progress_contract` peer link (issue #1041 — see [Cross-Contract Wiring](#cross-contract-wiring) below) |

### verification

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `contract_initialized` | event_name, admin (Address) | admin (Address) | Contract initialized |
| `admin_transfer_proposed` | event_name, old_admin (Address) | new_admin (Address) | Current admin proposes a replacement |
| `admin_transferred` | event_name, old_admin (Address) | new_admin (Address) | Pending admin accepts control |
| `milestone_approved` | event_name, validator (Address) | player_id (u64), milestone_index (u32), description (String), evidence_hash (String) | Validator confirms a player achievement |
| `validator_registered` | event_name, wallet (Address) | credentials (String) | New validator onboarded |
| `validator_revoked` | event_name, admin (Address) | wallet (Address), reason (String) | Validator deactivated |
| `validator_restored` | event_name, admin (Address) | wallet (Address) | Revoked validator re-activated |
| `validator_transferred` | event_name, admin (Address) | old_wallet (Address), new_wallet (Address) | Validator identity migrated to new wallet |
| `milestone_disputed` | event_name, player_wallet (Address) | player_id (u64), milestone_index (u32), reason (String) | Player disputes a milestone attribution |
| `dispute_resolved` | event_name, admin (Address) | player_id (u64), milestone_index (u32), upheld (bool) | Admin resolves a milestone dispute |
| `progress_contract_updated` | event_name, admin (Address) | progress_contract (Address) | Progress contract address re-wired |
| `contract_paused` | event_name, admin (Address) | () | Circuit breaker engaged |
| `contract_unpaused` | event_name, admin (Address) | () | Circuit breaker released |
| `attestation_recorded` | event_name, validator (Address) | player_id (u64), evidence_hash (String), vote_count (u32), threshold (u32) | `attest_milestone` vote accepted (including the threshold-crossing one) |
| `attestation_window_expired` | event_name, player_id (u64) | evidence_hash (String), new_round (u32) | A sub-threshold claim's voting window elapsed; the next vote starts a fresh round |
| `validator_votes_invalidated` | event_name, admin (Address) | wallet (Address), invalidated_count (u32) | `revoke_validator` retroactively stripped this validator's pending votes |
| `wiring_updated` | event_name, admin (Address), link (Symbol) | new_address (Address), new_epoch (u32) | `set_progress_contract` / `update_progress_contract` / `set_registration_contract` / `update_registration_contract` re-wired a peer link — `link` is `"progress_contract"` or `"registration_contract"` (issue #1041 — see [Cross-Contract Wiring](#cross-contract-wiring) below) |

### progress

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `progress_updated` | event_name, updated_by (Address) | player_id (u64), old_level, new_level | Player advances one level |
| `player_level_reset` | event_name, admin (Address) | player_id (u64), old_level, new_level | Admin resets a player's level |
| `admin_transfer_proposed` | event_name, old_admin (Address) | new_admin (Address) | Current admin proposes a replacement |
| `admin_transferred` | event_name, old_admin (Address) | new_admin (Address) | Pending admin accepts control |
| `contract_paused` | event_name, admin (Address) | () | Circuit breaker engaged |
| `contract_unpaused` | event_name, admin (Address) | () | Circuit breaker released |
| `wiring_updated` | event_name, admin (Address), link (Symbol) | new_address (Address), new_epoch (u32) | `set_registration_contract` / `set_verification_contract` / `set_scout_access_contract` re-wired a peer link — `link` is `"registration_contract"`, `"verification_contract"`, or `"scout_access_contract"` (issue #1041 — see [Cross-Contract Wiring](#cross-contract-wiring) below) |

### scout_access

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `contract_initialized` | event_name, admin (Address) | admin (Address) | Contract initialized |
| `scout_subscribed` | event_name, scout (Address) | tier (SubscriptionTier), fee_paid (i128) | Scout purchases a subscription (legacy; emitted alongside `subscription_created` or `subscription_renewed`) |
| `subscription_created` | event_name, scout (Address) | tier, subscribed_at (u64), expires_at (u64) | First-ever subscription for this scout (emitted alongside `scout_subscribed`) |
| `subscription_renewed` | event_name, scout (Address) | tier, subscribed_at (u64), expires_at (u64) | Existing subscription renewed or upgraded (emitted alongside `scout_subscribed`) |
| `player_contacted` | event_name, scout (Address) | player_id (u64), fee_paid (i128) | Scout unlocks player contact details |
| `trial_offer_logged` | event_name, scout (Address) | player_id (u64) | Elite scout records a trial offer |
| `trial_offer_confirmed` | event_name, scout (Address) | player_id (u64), index (u32) | Player confirms a pending trial offer before its expiry window closes; escrow released |
| `trial_offer_expired` | event_name, scout (Address) | player_id (u64), index (u32) | Trial offer confirmation window elapsed; escrowed fee refunded to scout |
| `fees_withdrawn` | event_name, admin (Address) | to (Address), amount (i128), timestamp (u64) | Admin withdraws accumulated fees |
| `subscription_refunded` | event_name, scout (Address) | amount (i128) | Admin issues emergency refund to a scout |
| `fee_config_updated` | event_name, admin (Address) | old_config (FeeConfig), new_config (FeeConfig) | Fee configuration changed (emitted by `update_fee_config`, `activate_fee_config`, and `propose_fee_config`'s immediate-decrease path) |
| `fee_config_proposed` | event_name, admin (Address) | proposed_config (FeeConfig), proposed_at (u64) | Admin proposes a fee change via `propose_fee_config` — always emitted; also accompanied by `fee_config_updated` in the same transaction if the proposal was an immediate decrease |
| `fee_config_delay_bypassed` | event_name, admin (Address) | old_config (FeeConfig), new_config (FeeConfig) | Emitted only by `update_fee_config`, alongside its own `fee_config_updated`, flagging that this fee change bypassed the 7-day `propose_fee_config`/`activate_fee_config` delay — see [`docs/FEE_CONFIG_PROPOSAL_DESIGN.md`](FEE_CONFIG_PROPOSAL_DESIGN.md#fee_config_delay_bypassed-new-1055) |
| `progress_contract_updated` | event_name, admin (Address) | progress_contract (Address) | Progress contract re-wired |
| `admin_transfer_proposed` | event_name, old_admin (Address) | new_admin (Address) | Current admin proposes a replacement |
| `admin_transferred` | event_name, old_admin (Address) | new_admin (Address) | Pending admin accepts control |
| `contract_paused` | event_name, admin (Address) | () | Circuit breaker engaged |
| `contract_unpaused` | event_name, admin (Address) | () | Circuit breaker released |
| `pay_to_contact_paused` | event_name, admin (Address) | () | Function-scoped circuit breaker for `pay_to_contact` engaged (issue #1056) |
| `pay_to_contact_unpaused` | event_name, admin (Address) | () | Function-scoped circuit breaker for `pay_to_contact` released |
| `wiring_updated` | event_name, admin (Address), link (Symbol) | new_address (Address), new_epoch (u32) | `set_progress_contract` / `update_progress_contract` / `set_registration_contract` re-wired a peer link — `link` is `"progress_contract"` or `"registration_contract"` (issue #1041 — see [Cross-Contract Wiring](#cross-contract-wiring) below) |
| `subscription_record_restored` | event_name, admin (Address) | scout (Address) | `restore_subscription_record` re-extends an archived or expired subscription entry's TTL back to the policy value |
| `evidence_access_granted` | event_name, scout (Address) | player_id (u64), tier_at_grant (SubscriptionTier) | `pay_to_contact` / `batch_contact_players` atomically authorizes this scout to request the wrapped decryption key for this player's evidence — see [EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md) |
| `evidence_access_revoked` | event_name, scout (Address) | player_id (u64), admin (Address) | `admin_revoke_evidence_access` — off-chain key-wrapping service should stop honoring *future* requests for this pair |

---

## Cross-Contract Wiring

See [`docs/WIRING_REGISTRY_DESIGN.md`](WIRING_REGISTRY_DESIGN.md) for the full design: every contract's `get_wiring_state()` getter, the `WiringLink { address, epoch }` shape shared via `scoutchain_shared_types`, the re-wiring policy (freely re-settable everywhere except verification's two legacy first-call-only setters, preserved for backward compatibility), and how `scripts/verify-cross-contract-wiring.sh` detects a partially-applied re-wiring across the eight peer-address pointers.

---

## Design Discussion: Check-Ordering Follow-ups

This section collects ordering decisions that were identified during the
check-precedence audit and flagged as candidates for review in a future
contract upgrade. None of these represent bugs in the current release —
all of them have documented, tested behavior — but some may produce a less
helpful error than a different ordering would. Each item describes the
current behavior, why it may be suboptimal, and the recommended change.

---

### 1. `log_trial_offer` is missing `require_initialized` — ✅ RESOLVED

**Resolved in**: PR feat/797-798-801-835 (full guard-ordering audit).

All state-changing functions in all four contracts — including `log_trial_offer`,
`register_player`, `update_profile`, `register_scout`, `admin_seed_player`,
`admin_seed_scout`, `register_validator`, `batch_register_validators`,
`approve_milestone`, `resolve_dispute`, and `reset_player_level` — now call
`require_not_paused` before `require_initialized`, matching the dominant
convention. `scripts/check-guard-ordering.sh` (wired into the CI lint job)
enforces this ordering automatically on every future PR.

---

### 2. `pay_to_contact`: `AlreadyContacted` checked before `ProContactLimitReached` (Priority 6 before 7)

**Current behavior**: The duplicate-contact guard (`AlreadyContacted`) runs
before the Pro monthly quota check (`ProContactLimitReached`). A scout who
is simultaneously at their quota limit *and* has already contacted the same
player sees `AlreadyContacted`.

**Rationale**: `AlreadyContacted` (error code 8) is the correct terminal error
for a genuine duplicate-contact attempt — it signals that this specific
`(scout, player_id)` pair has already been processed. The Pro-quota guard
at Priority 7 is a separate concern that only applies to new contacts; it
fires `ProContactLimitReached` (code 20) when a Pro-tier scout attempts to
contact a *new* player beyond their monthly limit. The ordering is therefore
correct for its intended purpose: duplicate detection takes precedence over
quota enforcement.

The apparent overlap — a Pro scout at their quota limit who re-attempts an
already-contacted player — sees `AlreadyContacted` because the duplicate-check
guard runs first. This is intentional: if a scout has already contacted a
player, the system should report that condition first, regardless of quota
status. The quota check is only meaningfully different for new contacts, where
it correctly returns `ProContactLimitReached`.

**Why no change is needed**: Swapping the ordering so that `ProContactLimitReached`
fires before `AlreadyContacted` would change the error semantics for any caller
that currently handles `AlreadyContacted` as the "duplicate already exists"
signal. The existing property-test suite (`check_precedence_property_tests.rs`)
locks in the current priority order across all reachable states, and altering
it would be a behavioral change beyond a simple doc update. The current ordering
is consistent, well-tested, and the quota-versus-duplicate overlap is rare in
practice because the quota guard is only active for new contacts.

**Decision**: No change. The ordering is correct and resolved.

---

### 3. `batch_contact_players` vs `pay_to_contact`: different error codes for the same quota limit — ✅ RESOLVED

**Resolved in**: v0.2.0 (scout_access). `batch_contact_players` now returns
`ProContactLimitReached` (20) instead of `ContactQuotaExceeded` (18). Code 18
is marked reserved/deprecated in `errors.rs`.

**What changed**: `check_pro_contact_quota_with_count` in
`contracts/scout_access/src/lib.rs` was updated to return
`ProContactLimitReached` (20), unifying both call paths on the same error
code. `ContactQuotaExceeded` (18) is retained in the enum with a deprecation
doc comment and its slot is reserved to prevent accidental reassignment.

**Impact**: Callers that previously matched `ContactQuotaExceeded` (18) from
`batch_contact_players` must update to `ProContactLimitReached` (20). This is
a MAJOR breaking change per `docs/VERSIONING.md` (error code removed/renamed).

---

### 4. `subscribe`: UpgradeTooSoon fires even for a same-tier renewal

**Current behavior**: the minimum 1-hour interval between `subscribe` calls
(the `UpgradeTooSoon` guard) applies to any call while the subscription is
active, including a renewal at exactly the same tier. A scout attempting to
renew their Pro subscription 30 minutes after purchasing it sees `UpgradeTooSoon`.

**Why this may be suboptimal**: The guard was introduced to prevent the
race-condition / double-charge scenario on rapid upgrades. A same-tier renewal
carries no race-condition risk because the tier does not change and the fee
is deterministic. Applying the interval guard to same-tier renewals is a
conservative over-application that can confuse users ("I'm just renewing,
why is it saying too soon?").

**Recommended fix**: Only apply the `UpgradeTooSoon` guard when the requested
tier is a strict upgrade (i.e., `tier_rank(&tier) > tier_rank(&existing.tier)`).
Same-tier renewals while active should only be rate-limited by the expiry
logic, not the upgrade interval. This is a small conditional change within the
existing `if now <= existing.expires_at` block.

**Risk**: Low. Removing the interval guard for same-tier renewals means two
identical-tier subscriptions *could* be purchased in rapid succession (paying
double). However, this is self-penalizing (the scout pays twice for no
benefit) and the new subscription simply overwrites the old one. The
`refund_subscription` admin function already handles the accidental-double-charge
recovery path.

# END OF DOCS CONTRACT_REFERENCE.md — all sections, TOC entries, and Design Discussion items are complete and resolved.

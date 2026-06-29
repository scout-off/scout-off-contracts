# Contract Reference

Complete public API reference for all four ScoutChain Soroban smart contracts.
Every `pub fn` in every `#[contractimpl]` block is documented here.

---

## Table of Contents

- [registration](#registration)
- [verification](#verification)
- [progress](#progress)
- [scout_access](#scout_access)
- [Shared Types](#shared-types)
- [Error Codes](#error-codes)
- [Events](#events)

---

## registration

Handles player and scout on-chain identity: registration, profile updates,
deregistration, and discovery queries.

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

#### `register_player(wallet: Address, vitals: PlayerVitals, ipfs_hashes: Vec<String>) -> Result<u64, ScoutChainError>`

Create a new on-chain player profile at Level 0 (Unverified).
Returns the assigned `player_id`.

| | |
|---|---|
| **Auth** | `wallet` must sign |
| **Errors** | `AlreadyRegistered` · `InvalidInput` (field too long or bad hash count) · `NotInitialized` · `ContractPaused` · `Overflow` |

Constraints:
- `position`, `region`, and `nationality` max 64 bytes each
- `ipfs_hashes` must contain 1–10 entries

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- register_player \
  --wallet $PLAYER_ADDRESS \
  --vitals '{"age":20,"position":"Forward","region":"West Africa","nationality":"Ghana"}' \
  --ipfs_hashes '["QmHighlightCID"]'
```

---

#### `update_profile(player_id: u64, ipfs_hashes: Vec<String>) -> Result<(), ScoutChainError>`

Replace a player's IPFS content hashes (highlight reels, photos).

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
  --region "West Africa"
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

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- set_progress_contract --addr $PROGRESS_CONTRACT_ID
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

#### `filter_players(region: String, position: String, min_level: ProgressLevel) -> Result<Vec<PlayerProfile>, ScoutChainError>`

Scout discovery query. Returns up to 50 player profiles matching the given
region, position, and minimum progress level.

Uses the composite `PlayersByLevelRegion(level, region)` index as the entry
point so only players that already satisfy the level+region criteria are loaded.
Gas cost is proportional to the number of matching players, not the total player
count. The index is maintained automatically on `register_player`,
`set_player_level`, and `deregister_player`.

| | |
|---|---|
| **Auth** | None |
| **Errors** | `NotInitialized` |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- filter_players \
  --region "West Africa" \
  --position "Forward" \
  --min_level '"Unverified"'
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

| | |
|---|---|
| **Auth** | None |
| **Errors** | `InvalidInput` (more than 20 IDs provided) |

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- get_players --ids '[1,2,3]'
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

### Dual-Role Wallet Policy

A single wallet may register as both a player and a scout. Cross-role
registration is permitted; duplicate prevention is enforced per role only.

---

## verification

Manages the trusted validator registry and milestone approvals. Cross-calls
`progress.advance_level` atomically when a milestone is approved.

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

#### `set_progress_contract(progress_contract: Address) -> Result<(), VerificationError>`

Wire the progress contract address so `approve_milestone` can call
`advance_level` cross-contract. Must be called once after deployment.
Returns `AlreadyConfigured` on subsequent calls — use
`update_progress_contract` for intentional re-wiring.

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
call. Use when redeploying or rotating the progress contract.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- update_progress_contract --progress_contract $NEW_PROGRESS_CONTRACT_ID
```

---

#### `register_validator(wallet: Address, credentials: String) -> Result<(), VerificationError>`

Onboard a new trusted validator (coach, academy director, certified trainer).
`credentials` is a human-readable label (max 256 bytes, e.g. `"UEFA B License"`).

The contract enforces a cap of **100 simultaneously registered validators**. This limit exists because all validator addresses are stored in a single persistent entry; exceeding Soroban's 64 KB per-entry limit would cause the entry to become unreadable. Raising the cap requires a contract upgrade.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorAlreadyRegistered` · `InvalidInput` (credentials >256 bytes) · `ValidatorCapReached` (100-validator limit reached) · `NotInitialized` · `ContractPaused` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- register_validator \
  --wallet $VALIDATOR_ADDRESS \
  --credentials "UEFA B License"
```

---

#### `revoke_validator(wallet: Address, reason: Option<String>) -> Result<(), VerificationError>`

Deactivate a validator. Revoked validators cannot approve milestones.
`reason` is optional and capped at 128 bytes.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `ValidatorNotFound` · `ReasonTooLong` (reason >128 bytes) · `Unauthorized` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- revoke_validator \
  --wallet $VALIDATOR_ADDRESS \
  --reason '"Misconduct"'
```

---

#### `approve_milestone(validator_wallet: Address, player_id: u64, description: String, evidence_hash: String) -> Result<u32, VerificationError>`

Record a verified milestone for a player. Caller must be a registered, active
validator. Evidence hash must be a valid IPFS (`Qm…`) or Arweave (`bafy…`) CID
of 2–128 bytes.

After storing the milestone this function cross-calls `progress.advance_level`
atomically so both state changes occur in the same Stellar transaction. Returns
the milestone index.

| | |
|---|---|
| **Auth** | `validator_wallet` must sign |
| **Errors** | `ContractPaused` · `ValidatorNotFound` · `ValidatorInactive` · `InvalidInput` (bad evidence hash) · `Overflow` · `ProgressCallFailed` |

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- approve_milestone \
  --validator_wallet $VALIDATOR_ADDRESS \
  --player_id 1 \
  --description "Scored 5 goals in Local Cup" \
  --evidence_hash "QmEvidence123"
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

Return the detailed status of a validator wallet: `Active`, `Revoked`, or
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
| `contract_initialized` | event_name | admin (Address) | Emitted on successful initialization |
| `milestone_approved` | event_name, validator_address, milestone_index (u32) | player_id (u64), description (String), evidence_hash (String) | Validator confirms a player achievement |
| `validator_registered` | event_name | validator_address | New validator onboarded |
| `validator_revoked` | event_name | validator_address, reason (String) | Validator deactivated |
| `progress_contract_updated` | event_name | new_address (Address) | Progress contract re-wired |
| `contract_paused` | event_name | admin (Address) | Circuit breaker engaged |
| `contract_unpaused` | event_name | admin (Address) | Circuit breaker released |

---

## progress

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

#### `transfer_admin(new_admin: Address) -> Result<(), ProgressError>`

Transfer admin rights to `new_admin`. The current admin loses **all** privileged
access immediately and irreversibly — there is no undo. The old admin address
is no longer authorised to call any admin-only function after this transaction
confirms.

> ⚠️ **Irreversible**: Once transferred, only `new_admin` can call
> `transfer_admin` again to change ownership. If `new_admin` is a lost or
> inaccessible key, admin access to this contract is permanently lost. Verify
> the new address before invoking.

**Parameters**

| Parameter | Type | Description |
|-----------|------|-------------|
| `new_admin` | `Address` | Stellar address that will become the new contract admin |

**Return type**: `Result<(), ProgressError>` — `Ok(())` on success.

| | |
|---|---|
| **Auth** | Current admin must sign (`require_auth` on the stored admin address) |
| **Errors** | `NotInitialized` if the contract has not been initialised |
| **Emits** | `admin_transferred` — topics: `(Symbol("admin_transferred"),)`, data: `(old_admin: Address, new_admin: Address)` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- transfer_admin --new_admin $NEW_ADMIN_ADDRESS
```

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

When the verification contract address is configured, only that contract may
invoke this function; otherwise `caller` must sign directly (useful for testing
without a full cross-contract deployment).

| | |
|---|---|
| **Auth** | Verification contract (production) or `caller` directly (test/unconfigured) |
| **Errors** | `NotInitialized` · `ContractPaused` · `AlreadyAtMaxLevel` · `Overflow` · `Unauthorized` |

_Called atomically by `verification.approve_milestone`. Prefer that path in production._

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
player IDs (no `PlayerNotFound` error).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_level --player_id 1
```

---

#### `get_history_count(player_id: u64) -> u32`

Return the total number of history entries recorded for a player.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_history_count --player_id 1
```

---

#### `get_history_entry(player_id: u64, index: u32) -> Result<ProgressEntry, ProgressError>`

Read a specific history entry. Indices start at `1`. Each `ProgressEntry`
includes `ledger_sequence: u32` for tamper-proof auditability.

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

Return all history entries for a player in chronological order. Internally reads
a single `HistoryVec` persistent storage key regardless of entry count — O(1)
reads instead of the previous O(N) loop. Returns an empty `Vec` for unknown
player IDs.

**Gas trade-off**: the Vec grows with each level change (max 3 entries per player
given the four-tier model). Because the entire Vec is loaded in one read the cost
is proportional to the serialised size of the Vec, not the number of reads.

**Migration note**: existing deployments that only have `HistoryEntry(player_id, i)`
keys (written before this change) will return an empty Vec from this function.
Use `get_history_entry` with individual indices to read pre-migration data, or
run a one-time migration script that calls `advance_level` / `reset_player_level`
to rewrite history into the new Vec key.

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

Store the verification contract address so `advance_level` can authenticate cross-contract callers. Without this, only direct `caller` auth is accepted (useful for testing). Admin only.

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

Store the registration contract address so `advance_level` can sync player levels via cross-contract call. Admin only.

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

Whitelist the scout_access contract as a secondary authorized caller of `advance_level` (for trial-offer Level-3 advances). Admin only.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- set_scout_access_contract --addr $SCOUT_ACCESS_CONTRACT_ID
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

Paginated history retrieval. Returns entries from `offset+1` to `offset+limit`. `limit` is capped at 50. Returns an empty `Vec` when `offset` >= total count.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- get_progress_history_page --player_id 1 --offset 0 --limit 10
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
| `player_level_reset` | event_name | player_id (u64), old_level, new_level | Admin resets a player's level |
| `admin_transferred` | event_name | old_admin (Address), new_admin (Address) | Admin rights rotated |

---

## scout_access

Handles scout subscriptions, pay-to-contact flows, and trial offer logging.
Fees are collected in XLM (stroops) and held in the contract until admin
withdrawal.

### Functions

---

#### `initialize(admin: Address, xlm_token: Address, fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

One-time contract setup. Validates that all fee fields are positive and
`sub_duration_secs` is non-zero.

| | |
|---|---|
| **Auth** | `admin` must sign |
| **Errors** | `AlreadyInitialized` · `InvalidInput` (zero or negative fee field) |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- initialize \
  --admin $ADMIN_ADDRESS \
  --xlm_token $XLM_TOKEN_ADDRESS \
  --fee_config '{"contact_fee_stroops":100000,"basic_sub_stroops":1000000,"pro_sub_stroops":3000000,"elite_sub_stroops":7000000,"sub_duration_secs":2592000}'
```

---

#### `transfer_admin(new_admin: Address) -> Result<(), ScoutAccessError>`

Transfer admin rights to a new address immediately.

| | |
|---|---|
| **Auth** | Current admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- transfer_admin --new_admin $NEW_ADMIN_ADDRESS
```

---

#### `set_progress_contract(addr: Address) -> Result<(), ScoutAccessError>`

Register the progress contract address so `log_trial_offer` can call
`advance_level` cross-contract (admin only).

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `NotInitialized` · `Unauthorized` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- set_progress_contract --addr $PROGRESS_CONTRACT_ID
```

---

#### `update_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

Adjust subscription and contact fee rates. Same validation rules as
`initialize`.

| | |
|---|---|
| **Auth** | Admin must sign |
| **Errors** | `Unauthorized` · `InvalidInput` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- update_fee_config \
  --fee_config '{"contact_fee_stroops":200000,"basic_sub_stroops":2000000,"pro_sub_stroops":5000000,"elite_sub_stroops":10000000,"sub_duration_secs":2592000}'
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
| **Errors** | `NotInitialized` · `ContractPaused` · `SubscriptionDowngradeNotAllowed` · `UpgradeTooSoon` · `Overflow` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- subscribe \
  --scout $SCOUT_ADDRESS \
  --tier '"Elite"'
```

---

#### `pay_to_contact(scout: Address, player_id: u64) -> Result<(), ScoutAccessError>`

Pay a micro-fee to unlock a player's contact details. Scout must have an active
(non-expired) subscription.

| | |
|---|---|
| **Auth** | `scout` must sign |
| **Errors** | `ContractPaused` · `ScoutNotSubscribed` · `SubscriptionExpired` · `AlreadyContacted` · `Overflow` |

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

| | |
|---|---|
| **Auth** | `scout` must sign |
| **Errors** | `ContractPaused` · `NotInitialized` · `ScoutNotSubscribed` · `SubscriptionExpired` · `Overflow` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- batch_contact_players \
  --scout $SCOUT_ADDRESS \
  --player_ids '[1,2,3]'
```

---

#### `log_trial_offer(scout: Address, player_id: u64, details_hash: String) -> Result<u32, ScoutAccessError>`

Record a trial offer on-chain. Scout must hold an active Elite subscription.
`details_hash` is an IPFS/Arweave CID of the offer document. Also calls
`progress.advance_level` if the progress contract is registered. Returns the
trial offer index.

| | |
|---|---|
| **Auth** | `scout` must sign (Elite subscription required) |
| **Errors** | `ContractPaused` · `ScoutNotSubscribed` · `SubscriptionExpired` · `Unauthorized` (non-Elite tier) · `Overflow` · `ProgressCallFailed` |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- log_trial_offer \
  --scout $SCOUT_ADDRESS \
  --player_id 1 \
  --details_hash "QmTrialOfferDetails"
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

#### `health() -> ContractHealth`

Return the contract's initialization and pause status.

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

---

#### `get_all_trial_offers(player_id: u64) -> Vec<TrialOffer>`

Return all trial offers for a player in a single call. Bounded at 20 to prevent gas exhaustion. Returns an empty `Vec` when no offers exist.

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- get_all_trial_offers --player_id 1
```

---

#### `version() -> String`

Return the deployed contract version string (from `Cargo.toml` at build time).

| | |
|---|---|
| **Auth** | None |
| **Errors** | None |

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID -- version
```

### Events

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `contract_initialized` | event_name, admin (Address) | admin (Address) | Emitted on successful initialization |
| `scout_subscribed` | event_name, scout (Address) | (tier: SubscriptionTier, fee_paid: i128) | Scout purchases a subscription |
| `player_contacted` | event_name, scout (Address) | (player_id: u64, fee_paid: i128) | Scout unlocks player contact details |
| `trial_offer_logged` | event_name, scout (Address) | player_id (u64) | Elite scout records a trial offer |
| `fees_withdrawn` | event_name, to (Address) | amount (i128) | Admin withdraws accumulated fees |
| `subscription_refunded` | event_name, scout (Address) | amount (i128) | Admin issues emergency refund to a scout |
| `admin_transferred` | event_name | (old_admin: Address, new_admin: Address) | Admin rights rotated |
| `contract_paused` | event_name | admin (Address) | Circuit breaker engaged |
| `contract_unpaused` | event_name | admin (Address) | Circuit breaker released |

---

## Shared Types

### `ProgressLevel`

Four-tier progress level used by all contracts.

| Integer | Variant | Meaning |
|---------|---------|---------|
| 0 | `Unverified` | Profile created, no verifications yet |
| 1 | `VerifiedIdentity` | Identity confirmed by a validator |
| 2 | `PerformanceMilestones` | Performance stats verified by a validator |
| 3 | `EliteTier` | Trial offer logged by an Elite scout |

Valid transitions: 0 → 1 → 2 → 3 (sequential only; no skipping or reversing
except via admin `reset_player_level`).

### `ContractHealth`

```rust
pub struct ContractHealth {
    pub initialized: bool,
    pub paused: bool,
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
    pub registered_at: u64,
    pub updated_at: u64,
}
```

### `ScoutProfile`

```rust
pub struct ScoutProfile {
    pub scout_id: u64,
    pub wallet: Address,
    pub region: String,   // max 128 bytes
    pub verified: bool,
    pub registered_at: u64,
}
```

### `Validator`

```rust
pub struct Validator {
    pub wallet: Address,
    pub credentials: String, // max 256 bytes
    pub registered_at: u64,
    pub active: bool,
}
```

### `ValidatorStatus`

```rust
pub enum ValidatorStatus {
    NotRegistered,
    Active,
    Revoked,
}
```

### `Milestone`

```rust
pub struct Milestone {
    pub player_id: u64,
    pub validator: Address,
    pub description: String,
    pub evidence_hash: String,  // IPFS Qm… or Arweave bafy…, 2–128 bytes
    pub approved_at: u64,
    pub ledger_sequence: u32,   // tamper-proof timestamp
}
```

### `ProgressEntry`

```rust
pub struct ProgressEntry {
    pub player_id: u64,
    pub old_level: ProgressLevel,
    pub new_level: ProgressLevel,
    pub updated_by: Address,
    pub updated_at: u64,
    pub milestone_ref: u32,     // links to verification contract index
    pub ledger_sequence: u32,   // tamper-proof timestamp
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
    pub expires_at: u64,
    pub subscribed_at: u64,
}
```

### `FeeConfig`

```rust
pub struct FeeConfig {
    pub contact_fee_stroops: i128,   // must be > 0
    pub basic_sub_stroops: i128,     // must be > 0
    pub pro_sub_stroops: i128,       // must be > 0
    pub elite_sub_stroops: i128,     // must be > 0
    pub sub_duration_secs: u64,      // must be > 0
}
```

### `TrialOffer`

```rust
pub struct TrialOffer {
    pub player_id: u64,
    pub scout: Address,
    pub details_hash: String, // IPFS/Arweave CID
    pub logged_at: u64,
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
| 9 | `InvalidInput` | Bad evidence hash or credentials too long |
| 10 | `ReasonTooLong` | Revocation reason exceeds 128 bytes |
| 11 | `AlreadyConfigured` | `set_progress_contract` called twice |
| 12 | `ProgressCallFailed` | Cross-contract `advance_level` failed |
| 13 | `Overflow` | Milestone counter overflowed |
| 14 | `MilestoneNotFound` | Index out of range |
| 15 | `ValidatorCapReached` | 100-validator limit reached; contract upgrade required to raise the cap |

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

### `ScoutAccessError` (scout_access contract)

| Code | Variant | Common Cause |
|------|---------|--------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotInitialized` | Operation before `initialize` |
| 3 | `ContractPaused` | Circuit breaker is active |
| 4 | `Unauthorized` | Wrong account or non-Elite tier for trial offer |
| 5 | `InsufficientFee` | Zero accumulated fees on withdrawal |
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

---

## Events

| Event | Contract | Emitted When |
|-------|----------|-------------|
| `player_registered` | registration | New player profile created |
| `scout_registered` | registration | New scout profile created |
| `profile_updated` | registration | Player updates IPFS content hashes |
| `player_deregistered` | registration | Admin removes a player profile |
| `scout_verified` | registration | Admin verifies a scout |
| `player_level_synced` | registration | Progress contract syncs a player's level |
| `contract_initialized` | verification | Contract initialized |
| `milestone_approved` | verification | Validator confirms a player achievement |
| `validator_registered` | verification | New validator onboarded |
| `validator_revoked` | verification | Validator deactivated |
| `progress_contract_updated` | verification | Progress contract address re-wired |
| `contract_paused` | verification / scout_access | Circuit breaker engaged |
| `contract_unpaused` | verification / scout_access | Circuit breaker released |
| `progress_updated` | progress | Player advances one level |
| `player_level_reset` | progress | Admin resets a player's level |
| `admin_transferred` | progress / scout_access | Admin rights rotated |
| `scout_subscribed` | scout_access | Scout purchases a subscription |
| `player_contacted` | scout_access | Scout unlocks player contact details |
| `trial_offer_logged` | scout_access | Elite scout records a trial offer |
| `fees_withdrawn` | scout_access | Admin withdraws accumulated fees |
| `subscription_refunded` | scout_access | Admin issues emergency refund to a scout |

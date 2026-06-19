# AI Integration Guide

Last reviewed: 2026-06-20
Reviewed against: `scout-off-contracts` contract workspace version `0.1.0`
at commit `bba2ac0` (`main` at review time).

This guide is for AI assistants and integration agents working across the
contracts, backend, indexer, and frontend repositories. Treat
`docs/CONTRACT_REFERENCE.md` and the Rust sources under `contracts/*/src` as the
source of truth when this guide and code drift.

## Repository Name

Use the current repository root:

```text
scout-off-contracts/
```

Do not use the older `scoutchain-contracts/` root path in generated commands,
documentation, issue text, or cross-repo integration notes.

## Contract Surface

The signatures below are copied from the current `#[contractimpl]` functions.
Keep the `Env` parameter in source-level references; generated client bindings
may hide it.

### registration

```rust
pub fn initialize(env: Env, admin: Address) -> Result<(), ScoutChainError>
pub fn pause_contract(env: Env) -> Result<(), ScoutChainError>
pub fn unpause_contract(env: Env) -> Result<(), ScoutChainError>
pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutChainError>
pub fn set_player_level(env: Env, player_id: u64, level: ProgressLevel) -> Result<(), ScoutChainError>
pub fn register_player(env: Env, wallet: Address, vitals: PlayerVitals, ipfs_hashes: Vec<String>) -> Result<u64, ScoutChainError>
pub fn update_profile(env: Env, player_id: u64, ipfs_hashes: Vec<String>) -> Result<(), ScoutChainError>
pub fn deregister_player(env: Env, player_id: u64) -> Result<(), ScoutChainError>
pub fn register_scout(env: Env, wallet: Address, region: String) -> Result<u64, ScoutChainError>
pub fn get_player(env: Env, player_id: u64) -> Result<PlayerProfile, ScoutChainError>
pub fn get_player_by_wallet(env: Env, wallet: Address) -> Result<PlayerProfile, ScoutChainError>
pub fn get_scout(env: Env, scout_id: u64) -> Result<ScoutProfile, ScoutChainError>
pub fn verify_scout(env: Env, scout_id: u64) -> Result<(), ScoutChainError>
pub fn get_player_count(env: Env) -> u64
pub fn get_scout_count(env: Env) -> u64
pub fn health(env: Env) -> ContractHealth
pub fn filter_players(env: Env, region: String, position: String, min_level: ProgressLevel) -> Result<Vec<PlayerProfile>, ScoutChainError>
```

### verification

```rust
pub fn initialize(env: Env, admin: Address) -> Result<(), VerificationError>
pub fn set_progress_contract(env: Env, progress_contract: Address) -> Result<(), VerificationError>
pub fn update_progress_contract(env: Env, progress_contract: Address) -> Result<(), VerificationError>
pub fn register_validator(env: Env, wallet: Address, credentials: String) -> Result<(), VerificationError>
pub fn get_validators(env: Env) -> Vec<Address>
pub fn revoke_validator(env: Env, wallet: Address, reason: Option<String>) -> Result<(), VerificationError>
pub fn pause_contract(env: Env) -> Result<(), VerificationError>
pub fn unpause_contract(env: Env) -> Result<(), VerificationError>
pub fn approve_milestone(env: Env, validator_wallet: Address, player_id: u64, description: String, evidence_hash: String) -> Result<u32, VerificationError>
pub fn get_milestone(env: Env, player_id: u64, index: u32) -> Result<Milestone, VerificationError>
pub fn get_milestone_count(env: Env, player_id: u64) -> u32
pub fn get_validator_milestone_count(env: Env, wallet: Address) -> u32
pub fn get_validator(env: Env, wallet: Address) -> Result<Validator, VerificationError>
pub fn get_validator_status(env: Env, wallet: Address) -> ValidatorStatus
pub fn is_active_validator(env: Env, wallet: Address) -> bool
pub fn health(env: Env) -> ContractHealth
```

### progress

```rust
pub fn initialize(env: Env, admin: Address) -> Result<(), ProgressError>
pub fn pause_contract(env: Env) -> Result<(), ProgressError>
pub fn unpause_contract(env: Env) -> Result<(), ProgressError>
pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), ProgressError>
pub fn reset_player_level(env: Env, player_id: u64, target_level: ProgressLevel) -> Result<(), ProgressError>
pub fn advance_level(env: Env, caller: Address, player_id: u64, milestone_ref: u32) -> Result<ProgressLevel, ProgressError>
pub fn get_level(env: Env, player_id: u64) -> ProgressLevel
pub fn get_history_count(env: Env, player_id: u64) -> u32
pub fn get_history_entry(env: Env, player_id: u64, index: u32) -> Result<ProgressEntry, ProgressError>
pub fn get_progress_history(env: Env, player_id: u64) -> Vec<ProgressEntry>
pub fn health(env: Env) -> ContractHealth
```

### scout_access

```rust
pub fn initialize(env: Env, admin: Address, xlm_token: Address, fee_config: FeeConfig) -> Result<(), ScoutAccessError>
pub fn update_fee_config(env: Env, fee_config: FeeConfig) -> Result<(), ScoutAccessError>
pub fn withdraw_fees(env: Env, to: Address) -> Result<i128, ScoutAccessError>
pub fn pause_contract(env: Env) -> Result<(), ScoutAccessError>
pub fn unpause_contract(env: Env) -> Result<(), ScoutAccessError>
pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutAccessError>
pub fn subscribe(env: Env, scout: Address, tier: SubscriptionTier) -> Result<(), ScoutAccessError>
pub fn pay_to_contact(env: Env, scout: Address, player_id: u64) -> Result<(), ScoutAccessError>
pub fn log_trial_offer(env: Env, scout: Address, player_id: u64, details_hash: String) -> Result<u32, ScoutAccessError>
pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), ScoutAccessError>
pub fn get_subscription(env: Env, scout: Address) -> Result<Subscription, ScoutAccessError>
pub fn get_fee_config(env: Env) -> FeeConfig
pub fn get_accumulated_fees(env: Env) -> i128
pub fn has_contacted(env: Env, scout: Address, player_id: u64) -> bool
pub fn get_trial_offer(env: Env, player_id: u64, index: u32) -> Result<TrialOffer, ScoutAccessError>
pub fn get_trial_count(env: Env, player_id: u64) -> u32
pub fn health(env: Env) -> ContractHealth
```

## Cross-Contract Wiring

Deploy and initialize all four contracts before wiring. After `.env.contracts`
exists and `ADMIN_ADDRESS`, `DEPLOYER_SECRET`, and `XLM_TOKEN_ADDRESS` are set,
the current required wiring is:

```bash
# registration trusts progress for player-level synchronization
stellar contract invoke \
  --id "$REGISTRATION_CONTRACT_ID" \
  --source "$DEPLOYER_SECRET" \
  --network "$NETWORK" \
  -- set_progress_contract \
  --addr "$PROGRESS_CONTRACT_ID"

# verification calls progress.advance_level after milestone approval
stellar contract invoke \
  --id "$VERIFICATION_CONTRACT_ID" \
  --source "$DEPLOYER_SECRET" \
  --network "$NETWORK" \
  -- set_progress_contract \
  --progress_contract "$PROGRESS_CONTRACT_ID"

# scout_access calls progress.advance_level after trial-offer logging
stellar contract invoke \
  --id "$SCOUT_ACCESS_CONTRACT_ID" \
  --source "$DEPLOYER_SECRET" \
  --network "$NETWORK" \
  -- set_progress_contract \
  --addr "$PROGRESS_CONTRACT_ID"
```

`verification.update_progress_contract(progress_contract)` is the supported
re-wiring path after an initial verification progress address has been set.
`registration` and `scout_access` currently expose only `set_progress_contract`.

## Error Codes

Decode errors by the contract that returned them. Numeric codes are not globally
unique across contracts.

### `ScoutChainError`

| Code | Variant |
| --- | --- |
| 1 | `AlreadyInitialized` |
| 2 | `NotInitialized` |
| 3 | `PlayerNotFound` |
| 4 | `ValidatorNotAuthorized` |
| 5 | `InvalidProgressTransition` |
| 6 | `ScoutNotSubscribed` |
| 7 | `InsufficientFee` |
| 8 | `AlreadyRegistered` |
| 9 | `ContractPaused` |
| 10 | `Unauthorized` |
| 11 | `Overflow` |
| 12 | `ScoutNotFound` |
| 13 | `InvalidInput` |

### `VerificationError`

| Code | Variant |
| --- | --- |
| 1 | `AlreadyInitialized` |
| 2 | `NotInitialized` |
| 3 | `ContractPaused` |
| 4 | `Unauthorized` |
| 5 | `ValidatorNotFound` |
| 6 | `ValidatorInactive` |
| 7 | `ValidatorAlreadyRegistered` |
| 8 | `PlayerNotFound` |
| 9 | `InvalidInput` |
| 10 | `ReasonTooLong` |
| 11 | `AlreadyConfigured` |
| 12 | `ProgressCallFailed` |
| 13 | `Overflow` |
| 14 | `MilestoneNotFound` |

### `ProgressError`

| Code | Variant |
| --- | --- |
| 1 | `AlreadyInitialized` |
| 2 | `NotInitialized` |
| 3 | `ContractPaused` |
| 4 | `Unauthorized` |
| 5 | `InvalidProgressTransition` |
| 6 | `AlreadyAtMaxLevel` |
| 7 | `PlayerNotFound` |
| 8 | `Overflow` |

### `ScoutAccessError`

| Code | Variant |
| --- | --- |
| 1 | `AlreadyInitialized` |
| 2 | `NotInitialized` |
| 3 | `ContractPaused` |
| 4 | `Unauthorized` |
| 5 | `InsufficientFee` |
| 6 | `ScoutNotSubscribed` |
| 7 | `SubscriptionExpired` |
| 8 | `AlreadyContacted` |
| 9 | `InvalidTier` |
| 10 | `Overflow` |
| 11 | `TrialOfferNotFound` |
| 12 | `SubscriptionDowngradeNotAllowed` |
| 14 | `ProgressCallFailed` |
| 15 | `InvalidInput` |
| 16 | `NoFeesToWithdraw` |

## Integration Notes

- `verification.revoke_validator` currently requires a third argument:
  `reason: Option<String>`.
- `registration.set_player_level` requires authorization from the stored
  progress contract address, not from the admin directly.
- `verification.approve_milestone` and `scout_access.log_trial_offer` both rely
  on the progress contract link before they can advance player levels.
- Regenerate bindings after deployment with `scripts/generate-bindings.sh` once
  `.env.contracts` contains all four contract IDs.

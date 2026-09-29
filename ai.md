# ScoutChain — AI Integration Guide

> **Last reviewed:** 2026-08-30
> **Contract version:** see `version()` on each deployed contract
> **Repo root:** `scout-off-contracts/`
>
> This document is kept in sync with the live `#[contractimpl]` signatures. If
> you spot a discrepancy, open an issue referencing this file and the affected
> function.

This document is the authoritative reference for AI assistants, SDK consumers,
and new team members integrating with ScoutChain's Soroban smart contracts.

---

## Contract Overview

| Contract | Package | Purpose |
|----------|---------|---------|
| `registration` | `scoutchain-registration` | Player & scout on-chain identity |
| `verification` | `scoutchain-verification` | Validator registry & milestone approvals |
| `progress` | `scoutchain-progress` | Four-tier level state machine |
| `scout_access` | `scoutchain-scout-access` | Subscriptions, pay-to-contact, trial offers |

---

## Shared Types

All four contracts import from `scoutchain-shared-types`:

```rust
pub enum ProgressLevel {
    Unverified,             // Level 0
    VerifiedIdentity,       // Level 1
    PerformanceMilestones,  // Level 2
    EliteTier,              // Level 3
}
```

The `registration` contract uses `PlayerVitals` as an input struct:

```rust
pub struct PlayerVitals {
    pub age:         u32,
    pub position:    String,  // max 64 bytes
    pub region:      String,  // max 128 bytes
    pub nationality: String,  // max 64 bytes
}
```

---

## Initialize Signatures

Each contract has a **one-time** `initialize` call. Calling it twice returns `AlreadyInitialized` (code 1).

```rust
// registration
pub fn initialize(env: Env, admin: Address) -> Result<(), ScoutChainError>

// verification
pub fn initialize(env: Env, admin: Address) -> Result<(), VerificationError>

// progress
pub fn initialize(env: Env, admin: Address) -> Result<(), ProgressError>

// scout_access — requires the XLM token address and an initial FeeConfig
pub fn initialize(
    env: Env,
    admin: Address,
    xlm_token: Address,
    fee_config: FeeConfig,
) -> Result<(), ScoutAccessError>
```

`scout_access.initialize` probes `xlm_token` by calling `decimals()` on it. A wrong address (testnet SAC on mainnet, a typo, a non-token contract) returns `InvalidInput` (code 15) immediately rather than failing on the first `subscribe()` call.

---

## Function Signatures — registration

```rust
pub fn register_player(
    env: Env,
    wallet: Address,
    vitals: PlayerVitals,
    ipfs_hashes: Vec<String>,
) -> Result<u64, ScoutChainError>

pub fn update_profile(
    env: Env,
    player_id: u64,
    ipfs_hashes: Vec<String>,
) -> Result<(), ScoutChainError>

pub fn register_scout(env: Env, wallet: Address, region: String) -> Result<u64, ScoutChainError>

pub fn filter_players(
    env: Env,
    region: String,
    position: String,
    min_level: ProgressLevel,
    offset: u32,
    limit: u32,
) -> Result<FilterResult, ScoutChainError>

// Queries
pub fn get_player(env: Env, player_id: u64) -> Result<PlayerProfile, ScoutChainError>
pub fn get_player_by_wallet(env: Env, wallet: Address) -> Result<PlayerProfile, ScoutChainError>
pub fn get_scout(env: Env, scout_id: u64) -> Result<ScoutProfile, ScoutChainError>
pub fn get_player_count(env: Env) -> u64
pub fn get_scout_count(env: Env) -> u64
pub fn health(env: Env) -> ContractHealth
pub fn version(env: Env) -> String

// Admin only
pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutChainError>
pub fn set_player_level(env: Env, player_id: u64, level: ProgressLevel) -> Result<(), ScoutChainError>
pub fn deregister_player(env: Env, player_id: u64) -> Result<(), ScoutChainError>
pub fn deactivate_player(env: Env, player_id: u64) -> Result<(), ScoutChainError>
pub fn reactivate_player(env: Env, player_id: u64) -> Result<(), ScoutChainError>
pub fn verify_scout(env: Env, scout_id: u64) -> Result<(), ScoutChainError>
pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), ScoutChainError>
pub fn accept_admin(env: Env) -> Result<(), ScoutChainError>
pub fn pause_contract(env: Env) -> Result<(), ScoutChainError>
pub fn unpause_contract(env: Env) -> Result<(), ScoutChainError>
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), ScoutChainError>
```

---

## Function Signatures — verification

```rust
pub fn register_validator(
    env: Env,
    wallet: Address,
    credentials: String,
    affiliation: String,
    specializations: Vec<String>,
) -> Result<(), VerificationError>

// `affiliation` is the canonical org identifier used for diversity gating;
// `specializations` are optional category tags such as "physical-stats" or "identity-kyc"
// that must match the milestone category when nested category gating is active.
pub fn revoke_validator(
    env: Env,
    wallet: Address,
    severity: RevocationSeverity,
    reason: Option<String>,
) -> Result<(), VerificationError>

pub fn batch_revoke_validators(
    env: Env,
    wallets: Vec<Address>,
    severity: RevocationSeverity,
    reason: Option<String>,
) -> Result<(), VerificationError>

pub fn batch_register_validators(
    env: Env,
    entries: Vec<(Address, String, String, Vec<String>)>,
) -> Result<(), VerificationError>

pub fn restore_validator(env: Env, wallet: Address) -> Result<(), VerificationError>

pub fn transfer_validator(
    env: Env,
    old_wallet: Address,
    new_wallet: Address,
) -> Result<(), VerificationError>

pub fn approve_milestone(
    env: Env,
    validator_wallet: Address,
    player_id: u64,
    description: String,
    evidence_hash: String,
    milestone_category: Option<String>,
) -> Result<u32, VerificationError>

// `milestone_category` is optional but when supplied, the validator must hold the
// matching specialization tag (returns `SpecializationMismatch` / code 21 otherwise).
// `approve_milestone` is also gated by the threshold-mode check: calling it after
// `set_milestone_threshold(n>=2)` returns `ThresholdModeRequiresAttestation`.

pub fn dispute_milestone(
    env: Env,
    player_wallet: Address,
    player_id: u64,
    milestone_index: u32,
    reason: String,
) -> Result<(), VerificationError>

pub fn resolve_dispute(
    env: Env,
    player_id: u64,
    milestone_index: u32,
    upheld: bool,
) -> Result<(), VerificationError>

// Queries
pub fn get_milestone(env: Env, player_id: u64, index: u32) -> Result<Milestone, VerificationError>
pub fn get_milestone_count(env: Env, player_id: u64) -> u32
pub fn get_validator(env: Env, wallet: Address) -> Result<Validator, VerificationError>
pub fn get_validators(env: Env) -> Vec<Address>
pub fn get_validator_milestone_count(env: Env, wallet: Address) -> u32
pub fn get_active_validator_count(env: Env) -> u32
pub fn is_active_validator(env: Env, wallet: Address) -> bool
pub fn health(env: Env) -> ContractHealth
pub fn version(env: Env) -> String

// Admin only
pub fn set_progress_contract(env: Env, progress_contract: Address) -> Result<(), VerificationError>
pub fn update_progress_contract(env: Env, progress_contract: Address) -> Result<(), VerificationError>
pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), VerificationError>
pub fn accept_admin(env: Env) -> Result<(), VerificationError>
pub fn pause_contract(env: Env) -> Result<(), VerificationError>
pub fn unpause_contract(env: Env) -> Result<(), VerificationError>
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), VerificationError>
```

---

## Function Signatures — progress

```rust
// Called cross-contract by verification (approve_milestone) and scout_access (confirm_trial_offer).
// Only the whitelisted verification and scout_access contract addresses may call this.
pub fn advance_level(
    env: Env,
    caller: Address,
    player_id: u64,
    milestone_ref: u32,
) -> Result<ProgressLevel, ProgressError>

pub fn reset_player_level(
    env: Env,
    player_id: u64,
    target_level: ProgressLevel,
) -> Result<(), ProgressError>

// Queries
pub fn get_level(env: Env, player_id: u64) -> ProgressLevel
pub fn get_history_count(env: Env, player_id: u64) -> u32
pub fn get_progress_history(env: Env, player_id: u64) -> Vec<ProgressEntry>
pub fn get_progress_history_page(env: Env, player_id: u64, offset: u32, limit: u32) -> Vec<ProgressEntry>
pub fn get_history_page_with_cursor(
    env: Env,
    player_id: u64,
    cursor_snapshot: Option<u32>,
    cursor_next_index: Option<u32>,
    limit: u32,
) -> (Vec<ProgressEntry>, u32, u32)
pub fn get_history_since(env: Env, player_id: u64, since_timestamp: u64) -> Vec<ProgressEntry>
pub fn health(env: Env) -> ContractHealth
pub fn version(env: Env) -> String

// Admin only — wiring setters (call once after deployment)
pub fn set_verification_contract(env: Env, addr: Address) -> Result<(), ProgressError>
pub fn set_registration_contract(env: Env, addr: Address) -> Result<(), ProgressError>
pub fn set_scout_access_contract(env: Env, addr: Address) -> Result<(), ProgressError>
pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), ProgressError>
pub fn accept_admin(env: Env) -> Result<(), ProgressError>
pub fn pause_contract(env: Env) -> Result<(), ProgressError>
pub fn unpause_contract(env: Env) -> Result<(), ProgressError>
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), ProgressError>
```

---

## Function Signatures — scout_access

```rust
pub fn subscribe(
    env: Env,
    scout: Address,
    tier: SubscriptionTier,   // Basic | Pro | Elite
) -> Result<(), ScoutAccessError>

pub fn pay_to_contact(
    env: Env,
    scout: Address,
    player_id: u64,
) -> Result<(), ScoutAccessError>

// Trial offer is a two-step flow:
// Step 1 — Elite scout logs the offer (scout must have previously contacted the player)
pub fn log_trial_offer(
    env: Env,
    scout: Address,
    player_id: u64,
    details_hash: String,   // IPFS CID
) -> Result<u32, ScoutAccessError>   // returns trial index

// Step 2 — Player confirms; escrow released, player advances to Level 3
pub fn confirm_trial_offer(
    env: Env,
    player_wallet: Address,
    player_id: u64,
    index: u32,
    idempotency_nonce: Option<String>,
) -> Result<(), ScoutAccessError>

// Queries
pub fn get_subscription(env: Env, scout: Address) -> Result<Subscription, ScoutAccessError>
pub fn get_fee_config(env: Env) -> FeeConfig
pub fn has_contacted(env: Env, scout: Address, player_id: u64) -> bool
pub fn get_trial_offer(env: Env, player_id: u64, index: u32) -> Result<TrialOffer, ScoutAccessError>
pub fn get_trial_count(env: Env, player_id: u64) -> u32
pub fn health(env: Env) -> ContractHealth
pub fn version(env: Env) -> String

// Admin only
pub fn update_fee_config(env: Env, fee_config: FeeConfig) -> Result<(), ScoutAccessError>
pub fn withdraw_fees(env: Env, to: Address) -> Result<i128, ScoutAccessError>
pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutAccessError>
pub fn update_progress_contract(env: Env, addr: Address) -> Result<(), ScoutAccessError>
pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), ScoutAccessError>
pub fn accept_admin(env: Env) -> Result<(), ScoutAccessError>
pub fn pause_contract(env: Env) -> Result<(), ScoutAccessError>
pub fn unpause_contract(env: Env) -> Result<(), ScoutAccessError>
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), ScoutAccessError>
```

---

## Cross-Contract Wiring

Eight peer-address links must be established after every fresh deployment. `initialize.sh` sets all eight automatically. Run the diagnostic script to check which links are present:

```bash
./scripts/verify-cross-contract-wiring.sh testnet
```

### The eight wiring links

| # | Command | What it does |
|---|---------|-------------|
| 1 | `verification.set_progress_contract` | Allows `approve_milestone` to call `advance_level` |
| 2 | `verification.set_registration_contract` | Allows the dispute-milestone wallet-to-`player_id` binding check |
| 3 | `registration.set_progress_contract` | Lets `filter_players` resolve player levels at query time |
| 4 | `progress.set_verification_contract` | Whitelists verification as authorized caller of `advance_level` |
| 5 | `progress.set_registration_contract` | Allows progress to call `set_player_level` on registration |
| 6 | `progress.set_scout_access_contract` | Whitelists scout_access as authorized caller of `advance_level` |
| 7 | `scout_access.set_progress_contract` | Allows `confirm_trial_offer` to call `advance_level` for Level 3 |
| 8 | `scout_access.set_registration_contract` | Pro-tier scout verification / Sybil gating lookups |

This list matches what `scripts/verify-cross-contract-wiring.sh` checks and the "Full Picture" table in [`docs/WIRING_REGISTRY_DESIGN.md`](docs/WIRING_REGISTRY_DESIGN.md).

### Manual wiring commands

```bash
# 1. Verification → Progress
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_progress_contract --progress_contract $PROGRESS_CONTRACT_ID

# 2. Verification → Registration
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_registration_contract --reg_contract $REGISTRATION_CONTRACT_ID

# 3. Registration → Progress
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_progress_contract --addr $PROGRESS_CONTRACT_ID

# 4. Progress → Verification
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_verification_contract --addr $VERIFICATION_CONTRACT_ID

# 5. Progress → Registration
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_registration_contract --addr $REGISTRATION_CONTRACT_ID

# 6. Progress → Scout Access
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_scout_access_contract --addr $SCOUT_ACCESS_CONTRACT_ID

# 7. Scout Access → Progress
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_progress_contract --addr $PROGRESS_CONTRACT_ID

# 8. Scout Access → Registration
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_registration_contract --addr $REGISTRATION_CONTRACT_ID
```

> **Note:** `verification.set_progress_contract` is first-call-only and returns
> `AlreadyConfigured` (code 11) on subsequent calls. Use
> `update_progress_contract` to re-wire after redeployment.
> All other wiring setters can be called repeatedly.

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `REGISTRATION_CONTRACT_ID` | Deployed registration contract ID |
| `VERIFICATION_CONTRACT_ID` | Deployed verification contract ID |
| `PROGRESS_CONTRACT_ID` | Deployed progress contract ID |
| `SCOUT_ACCESS_CONTRACT_ID` | Deployed scout_access contract ID |

---

## TypeScript Bindings

After deployment run `./scripts/generate-bindings.sh <network>`. Import the generated clients:

```typescript
import { Client as RegistrationClient } from "@scoutchain/bindings-registration";
import { Client as VerificationClient }  from "@scoutchain/bindings-verification";
import { Client as ProgressClient }      from "@scoutchain/bindings-progress";
import { Client as ScoutAccessClient }   from "@scoutchain/bindings-scout-access";
```

See `bindings/README.md` for full usage details.

---

## Error Codes

Error codes are **per-contract**. The same numeric code can mean different things in different contracts. Always check which contract returned the error.

### `ScoutChainError` (registration)

| Code | Variant | Cause |
|------|---------|-------|
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
| 14 | `PendingAdminNotSet` | `accept_admin` called without a prior `propose_admin` |
| 15 | `PlayerCapReached` | Player registration cap reached |
| 16 | `RegistrationCooldown` | Caller attempted to register again before cooldown |

> **Note:** `RegistrationCooldown` (code 16) differs from the same-named code in the verification contract (code 25, see below). Always check which contract returned the error.

| 17 | `PlayerRecordEvicted` | Archival grace period fully elapsed, unrecoverable |
| 18 | `ScoutRecordEvicted` | Scout record fully evicted, unrecoverable |

### `VerificationError` (verification)

| Code | Variant | Cause |
|------|---------|-------|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotInitialized` | Operation before `initialize` |
| 3 | `ContractPaused` | Circuit breaker is active |
| 4 | `Unauthorized` | Wrong account for a privileged operation |
| 5 | `ValidatorNotFound` | Wallet not in validator registry |
| 6 | `ValidatorInactive` | Validator has been revoked |
| 7 | `ValidatorAlreadyRegistered` | Wallet already registered as validator |
| 8 | `PlayerNotFound` | Invalid `player_id` |
| 9 | `InvalidInput` | Bad evidence hash or credentials too long/short |
| 10 | `ReasonTooLong` | Revocation reason exceeds 128 bytes |
| 11 | `AlreadyConfigured` | `set_progress_contract` called twice; use `update_progress_contract` |
| 12 | `ProgressCallFailed` | Cross-contract `advance_level` failed |
| 13 | `Overflow` | Milestone counter overflowed |
| 14 | `MilestoneNotFound` | Index out of range |
| 15 | `ValidatorCapReached` | 100-validator platform limit reached |
| 16 | `DuplicateEvidence` | Evidence hash already used in a prior approval |
| 17 | `MilestoneLimitExceeded` | Validator has already approved 5 milestones for this player |
| 18 | `DisputeAlreadyResolved` | Dispute was already resolved |
| 19 | `PendingAdminNotSet` | `accept_admin` called without a prior `propose_admin` |
| 20 | `ApproveMilestonePaused` | `approve_milestone` function is paused independently |
| 21 | `SpecializationMismatch` | Validator not tagged for requested milestone category |
| 22 | `InvalidAttestation` | ed25519 signature verification failed |
| 23 | `AttestationKeyNotFound` | No attestation public key registered |
| 24 | `InvalidNonce` | Nonce not strictly greater than last accepted |
| 25 | `RegistrationCooldown` | Validator registration before cooldown window elapsed |
| 26 | `DuplicateAttestation` | Same validator already attested to this claim in current round |
| 27 | `TooManyPendingVotes` | Validator has MAX_PENDING_VOTES_PER_VALIDATOR outstanding votes |
| 28 | `ThresholdModeRequiresAttestation` | threshold >= 2, must use attest_milestone bypass |
| 29 | `MigrationNotActive` | Migration window not currently active |
| 30 | `MilestoneAlreadyExists` | Milestone already exists at (player_id, milestone_index) with different content |
| 31 | `DisputeAlreadyExists` | Dispute already exists at (player_id, milestone_index) with different content |
| 32 | `ValidatorRecordEvicted` | Validator record fully evicted, unrecoverable |
| 33 | `MilestoneRecordEvicted` | Milestone record fully evicted, unrecoverable |
| 34 | `NotEligibleToReReview` | Caller is not a currently-active validator |
| 35 | `MilestoneNotFlagged` | Milestone not currently flagged as pending re-review |
| 36 | `DisputeRequiresJury` | resolve_dispute called on a dispute requiring jury resolution |
| 37 | `NotJuryDispute` | cast_dispute_vote/tally_dispute called on non-jury dispute |
| 38 | `VotingWindowClosed` | cast_dispute_vote called after voting window closed |
| 39 | `ConflictOfInterest` | cast_dispute_vote called by the validator who approved the disputed milestone |
| 40 | `AlreadyVoted` | cast_dispute_vote called by a validator who already voted on this dispute |
| 41 | `VotingWindowOpen` | tally_dispute called before voting window closes, vote count is tied |
| 42 | `QuorumNotReached` | tally_dispute called before quorum of votes has been reached |

### `ProgressError` (progress)

| Code | Variant | Cause |
|------|---------|-------|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotInitialized` | Operation before `initialize` |
| 3 | `ContractPaused` | Circuit breaker is active |
| 4 | `Unauthorized` | Caller is not the whitelisted verification or scout_access contract |
| 5 | `InvalidProgressTransition` | Level skip or reversal attempted |
| 6 | `AlreadyAtMaxLevel` | Player is already at `EliteTier` |
| 7 | `PlayerNotFound` | History index out of range |
| 8 | `Overflow` | History counter overflowed the maximum u32 value |
| 9 | `RegistrationCallFailed` | Cross-contract call to registration contract failed |
| 10 | `PendingAdminNotSet` | `accept_admin` called without a prior `propose_admin` |
| 11 | `MigrationNotActive` | Migration window not currently active |
| 12 | `HistoryAlreadyExists` | HistoryEntry already exists at (player_id, history_index) with different content |
| 13 | `MerkleRootMismatch` | Independently recomputed Merkle root doesn't match expected root |
| 14 | `InvalidHistoryIndex` | Supplied history_index is zero, non-contiguous, or would overwrite existing entry |
| 15 | `PlayerLevelRecordEvicted` | Player-level record fully evicted, unrecoverable |

### `ScoutAccessError` (scout_access)

| Code | Variant | Cause |
|------|---------|-------|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotInitialized` | Operation before `initialize` |
| 3 | `ContractPaused` | Circuit breaker is active |
| 4 | `Unauthorized` | Caller not authorized; or non-Elite tier for `log_trial_offer` |
| 5 | `InsufficientFee` | Scout underpaid a subscription or contact fee |
| 6 | `ScoutNotSubscribed` | No subscription record found |
| 7 | `SubscriptionExpired` | Subscription past `expires_at` |
| 8 | `AlreadyContacted` | Duplicate `pay_to_contact` for same player |
| 9 | `InvalidTier` | Unknown subscription tier |
| 10 | `Overflow` | Fee accumulation arithmetic overflowed |
| 11 | `TrialOfferNotFound` | Trial offer index out of range |
| 12 | `SubscriptionDowngradeNotAllowed` | Downgrade attempted while subscription is active |
| 13 | **Reserved** | Code 13 is intentionally reserved and must not be assigned; it is held open to prevent future contributors from accidentally colliding with any external consumers that may already treat 13 as an expected (if undocumented) gap. See docs/VERSIONING.md — error-code compatibility. |
| 14 | `ProgressCallFailed` | Cross-contract `advance_level` failed in `confirm_trial_offer` |
| 15 | `InvalidInput` | Zero/negative fee field in `FeeConfig`, or bad token address in `initialize` |
| 16 | `NoFeesToWithdraw` | No accumulated fees to withdraw |
| 17 | `UpgradeTooSoon` | `subscribe` called before 1-hour minimum interval elapsed |
| 18 | `ContactQuotaExceeded` | Platform-wide contact quota for current period hit |
| 19 | `TrialOfferRateLimited` | Trial offer to same player within cooldown window |
| 20 | `ProContactLimitReached` | Pro-tier scout hit per-period contact limit |
| 21 | `PendingAdminNotSet` | `accept_admin` called without a prior `propose_admin` |
| 22 | `TrialOfferAlreadyConfirmed` | `confirm_trial_offer` called twice for same offer |
| 23 | `TrialOfferExpired` | Legacy compatibility code; expiry confirmation now commits the refund and returns success |
| 24 | `NoPendingFeeConfig` | No pending fee config proposal exists for `activate_fee_config` |
| 25 | `FeeConfigProposalNotReady` | Pending fee config proposal activation delay has not yet elapsed |
| 26 | `PendingFeeConfigAlreadyExists` | A fee config proposal already exists; must activate or replace it |
| 27 | `ScoutNotVerified` | Scout is not verified; cannot subscribe to Pro tier |
| 28 | `AutoRenewNotEnabled` | `renew_if_due` called but auto-renewal is not enabled |
| 29 | `MigrationNotActive` | Migration window not currently active |
| 30 | `SubscriptionAlreadyExists` | Subscription already exists for this scout address with different content |
| 31 | `ContactAlreadyExists` | ContactRecord already exists for (player_id, scout) with different content |
| 32 | `TrialOfferAlreadyExists` | TrialOffer already exists at (player_id, trial_index) with different content |
| 33 | `AutoRenewAlreadyExists` | AutoRenew flag already exists for this scout with a different value |
| 34 | `FeeConfigHistoryAlreadyExists` | Fee-config history replay conflicts with history already stored |
| 35 | `SubscriptionRecordEvicted` | Subscription record fully evicted, unrecoverable |
| 36 | `PayToContactPaused` | `pay_to_contact` function is paused independently of whole-contract pause |
| 37 | `TrialEscrowNotOutstanding` | `admin_refund_trial_escrow` targeted pair with no outstanding `TrialEscrow` entry |
| 38 | `GrantNotFound` | `admin_revoke_evidence_access` targeted (player_id, scout) pair with no `EvidenceAccessGrant` record |

---

## Error Handling — ProgressCallFailed

### What it is

`ProgressCallFailed` is returned by two contracts when the cross-contract call to `progress.advance_level` fails at runtime:

| Contract | Error enum | Code |
|----------|-----------|------|
| `verification` | `VerificationError::ProgressCallFailed` | 12 |
| `scout_access` | `ScoutAccessError::ProgressCallFailed` | 14 |

### When it is returned

- **`verification.approve_milestone`** — after writing the milestone to storage, calls `progress.advance_level`. Any failure other than `AlreadyAtMaxLevel` returns `ProgressCallFailed` and reverts the entire transaction.
- **`scout_access.confirm_trial_offer`** — after verifying the offer record, calls `progress.advance_level` to advance the player to Level 3. Any failure returns `ProgressCallFailed` and reverts the transaction.

### Diagnostic events (new)

Before returning `ProgressCallFailed`, both contracts now emit a `progress_call_failed` diagnostic event with the raw error discriminant so indexers can detect failures by scanning receipts without parsing error codes.

When `AlreadyAtMaxLevel` is silently skipped in `approve_milestone`, a `level_advancement_skipped` event is emitted with `reason = "AlreadyAtMaxLevel"`.

When the progress contract is not wired, a `progress_contract_not_set` event is emitted (committed to the ledger) before returning.

### Recovery steps

1. Run the wiring diagnostic:
   ```bash
   ./scripts/verify-cross-contract-wiring.sh testnet
   ```
   It checks all eight documented wiring links: `verification.set_progress_contract`, `verification.set_registration_contract`, `registration.set_progress_contract`, `progress.set_verification_contract`, `progress.set_registration_contract`, `progress.set_scout_access_contract`, `scout_access.set_progress_contract`, and `scout_access.set_registration_contract`.
2. Re-wire if any link shows ❌:
   ```bash
   ./scripts/initialize.sh testnet
   ```
3. Retry the original transaction — because `ProgressCallFailed` aborts the whole transaction, there is no partial state to clean up.

> **Retry only the original entry point, never `advance_level` directly (Issue #811 follow-up).**
> `progress.advance_level` is **not** internally idempotent. It is a monotonic state-machine step: it reads the current level, computes `next()`, and appends a history entry. It does **not** key off `milestone_ref`, so calling it twice with the *same* `milestone_ref` advances two tiers and writes two history entries that are indistinguishable from a legitimate double advance.
>
> Retry is safe **only** because each production caller holds its own dedup key and the whole transaction reverts on failure:
> - `verification.approve_milestone` → `DataKey::EvidenceUsed(evidence_hash)` (returns `DuplicateEvidence`, code 16)
> - `scout_access.confirm_trial_offer` → `DataKey::ConfirmationNonce(...)` / absent `TrialEscrow` (returns `TrialOfferAlreadyConfirmed`, code 22)
>
> Because the failed attempt reverts, its dedup key is rolled back too, so the retry proceeds exactly once. An operator or new contract that calls `progress.advance_level` directly gets **no such protection** and must supply its own dedup key. Any new whitelisted caller of `advance_level` must do the same.
>
> Bounded blast radius: a replay can over-advance by at most three tiers; once at `EliteTier` further calls fail closed with `AlreadyAtMaxLevel` and write nothing. On the secondary (`scout_access`) path, a `milestone_ref` of `0` or one beyond the verification contract's real milestone count is rejected with `InvalidProgressTransition` (#457). Both are verified in `contracts/progress/tests/issue_811_idempotency.rs`.

> **Tested guarantee (Issue #811):** This all-or-nothing claim is backed by adversarial tests, not just this prose explanation. See:
> - `contracts/verification/tests/adversarial_atomicity.rs` — proves `approve_milestone` behavior on bad-wired progress contract and validates the `DuplicateEvidence` idempotency token as defense-in-depth.
> - `contracts/scout_access/tests/adversarial_atomicity.rs` — equivalent tests for `confirm_trial_offer`, including the `TrialOfferAlreadyConfirmed` double-confirm guard.
> - `contracts/progress/tests/issue_811_idempotency.rs` — audits the shared `advance_level` call target itself: pins the non-idempotent double-apply behaviour, proves rejected calls leave no partial state, and proves the `AlreadyAtMaxLevel` / `InvalidProgressTransition` fail-closed paths.
>
> **Idempotency defense-in-depth:** The `DuplicateEvidence` check (code 16) on `approve_milestone` acts as an explicit idempotency token: if a future refactor altered write ordering and partial state were committed, a retried call with the same evidence hash would return `DuplicateEvidence` rather than silently double-counting. For `confirm_trial_offer`, the absence of the `TrialEscrow` record (removed on first confirmation) serves the same purpose — a second call returns `TrialOfferAlreadyConfirmed` (code 22).

---

## Events Reference

### verification

| Event | Topics | Data | Committed? |
|-------|--------|------|-----------|
| `contract_initialized` | event_name | admin (Address) | ✅ |
| `milestone_approved` | event_name, validator (Address), milestone_index (u32) | player_id (u64), description (String), evidence_hash (String) | ✅ |
| `validator_registered` | event_name, wallet (Address) | wallet (Address), credentials (String) | ✅ |
| `validator_revoked` | event_name | wallet (Address), reason (String) | ✅ |
| `validator_votes_invalidated` | event_name | wallet (Address) | ✅ |
| `validator_restored` | event_name | wallet (Address) | ✅ |
| `validator_transferred` | event_name | old_wallet (Address), new_wallet (Address) | ✅ |
| `milestone_flagged` | event_name, player_id (u64), milestone_index (u32) | () | ✅ |
| `milestone_disputed` | event_name, player_id (u64), milestone_index (u32) | reason (String) | ✅ |
| `dispute_resolved` | event_name, player_id (u64), milestone_index (u32) | upheld (bool) | ✅ |
| `progress_contract_updated` | event_name | new_address (Address) | ✅ |
| `contract_paused` | event_name | admin (Address) | ✅ |
| `contract_unpaused` | event_name | admin (Address) | ✅ |
| `admin_transfer_proposed` | event_name | old_admin (Address), new_admin (Address) | ✅ |
| `admin_transferred` | event_name | old_admin (Address), new_admin (Address) | ✅ |
| `level_advancement_skipped` | event_name, player_id (u64) | reason (String) | ✅ |
| `progress_contract_not_set` | event_name, player_id (u64) | `()` | ✅ |
| `progress_call_failed` | event_name, player_id (u64) | error_code (u32) | ⚠️ diagnostic only |

### progress

| Event | Topics | Data | Committed? |
|-------|--------|------|-----------|
| `progress_updated` | event_name, updated_by (Address) | player_id (u64), old_level, new_level | ✅ |
| `player_level_reset` | event_name | player_id (u64), old_level, new_level | ✅ |
| `contract_paused` | event_name | admin (Address) | ✅ |
| `contract_unpaused` | event_name | admin (Address) | ✅ |
| `admin_transfer_proposed` | event_name | old_admin (Address), new_admin (Address) | ✅ |
| `admin_transferred` | event_name | old_admin (Address), new_admin (Address) | ✅ |

### scout_access

| Event | Topics | Data | Committed? |
|-------|--------|------|-----------|
| `contract_initialized` | event_name, admin (Address) | admin (Address) | ✅ |
| `scout_subscribed` | event_name, scout (Address) | tier (SubscriptionTier), fee_paid (i128) | ✅ |
| `subscription_created` | event_name, scout (Address) | tier (SubscriptionTier), subscribed_at (u64), expires_at (u64) | ✅ |
| `subscription_renewed` | event_name, scout (Address) | tier (SubscriptionTier), subscribed_at (u64), expires_at (u64) | ✅ |
| `player_contacted` | event_name, scout (Address) | player_id (u64), fee_paid (i128) | ✅ |
| `trial_offer_logged` | event_name, scout (Address) | player_id (u64) | ✅ |
| `trial_offer_confirmed` | event_name, scout (Address) | player_id (u64), index (u32) | ✅ |
| `trial_offer_expired` | event_name, scout (Address) | player_id (u64), index (u32) | ✅ |
| `fees_withdrawn` | event_name | to (Address), amount (i128) | ✅ |
| `fee_config_updated` | event_name | old_config (FeeConfig), new_config (FeeConfig) | ✅ |
| `subscription_refunded` | event_name, scout (Address) | amount (i128) | ✅ |
| `contract_paused` | event_name | admin (Address) | ✅ |
| `contract_unpaused` | event_name | admin (Address) | ✅ |
| `admin_transfer_proposed` | event_name | old_admin (Address), new_admin (Address) | ✅ |
| `admin_transferred` | event_name | old_admin (Address), new_admin (Address) | ✅ |
| `progress_contract_not_set` | event_name, player_id (u64) | `()` | ✅ |
| `progress_call_failed` | event_name, player_id (u64) | error_code (u32) | ⚠️ diagnostic only |

> **Diagnostic-only events** appear in the transaction receipt's diagnostic stream but are not committed to the ledger. Indexers must scan receipts rather than the standard Horizon event stream to capture them.

---

## Common Integration Pitfalls

- **Wiring must be re-run after every fresh deployment.** Contract IDs change on each deploy; old wiring references stale IDs.
- **`initialize` is one-time per contract.** Calling it twice returns `AlreadyInitialized` (code 1). This is not an error — the contract is already ready.
- **`revoke_validator` now takes an explicit `RevocationSeverity` as the second parameter.** Pass `RevocationSeverity::Routine` for a routine deactivation (no cascade) or `RevocationSeverity::ForCause` for a misconduct revocation that flags all prior milestone approvals as pending re-review. Pass `None` for reason if no reason is needed. The old single-`reason` signature no longer exists — update all callers. For validators with more than 50 prior approvals, call `continue_revocation_cascade(wallet)` (admin only) one or more times until the `revocation_cascade_complete` event is emitted.
- **`log_trial_offer` does NOT immediately advance the player's level.** It records the offer and escrows a fee. Level advancement happens in `confirm_trial_offer`, which must be called by the player wallet.
- **Trial offer two-step flow:** `log_trial_offer` (scout) → `confirm_trial_offer` (player). Missing the confirmation step means the player stays at Level 2.
- **Admin rotation is two-step.** Current admin calls `propose_admin`, then the pending address calls `accept_admin`. The old admin remains active until acceptance.
- **Error codes are per-contract, not global.** Code `4` means `Unauthorized` in verification but also `Unauthorized` (different context) in scout_access. Code `9` means `ContractPaused` in registration but `RegistrationCallFailed` in progress. Always check which contract returned the error.
- **Subscription tier check is enforced on-chain.** Basic scouts cannot call `pay_to_contact`. Elite is required for `log_trial_offer`.
- **`filter_players` requires `offset` and `limit`.** The limit is capped at 50 server-side.
- **`set_progress_contract` on verification is first-call-only.** Returns `AlreadyConfigured` (code 11) if called again. Use `update_progress_contract` to re-wire.
- **`approve_milestone` stops working once k-of-n threshold mode is enabled.** Once an admin calls `set_milestone_threshold(n)` with `n >= 2`, both `approve_milestone` and `submit_attested_milestone` return `ThresholdModeRequiresAttestation` (code 28) for every subsequent call — all milestone submissions must go through `attest_milestone` instead. Call `get_milestone_threshold()` to check the current mode before integrating; a return value of `1` (the default) means single-signature mode is still active and `approve_milestone` works as normal. A return value of `2` or higher means every validator must call `attest_milestone` independently, and the milestone commits automatically once the threshold number of distinct active validators have voted for the same `(player_id, evidence_hash)` claim within the configured voting window.

> **⚠️ Verify `log_trial_offer` behavior against the live contract before integrating.**
> The documented two-step flow (`log_trial_offer` → `confirm_trial_offer`) and level-advancement mechanics in this file reflect the contract's *intended* behavior at the time of writing. However, on-chain behavior is the ultimate source of truth. Before building any integration that depends on trial offers, **call `log_trial_offer` on the target network (testnet/mainnet) with a test scout account and inspect the resulting transaction: confirm the offer is recorded, the fee is escrowed, and no level advancement occurs until `confirm_trial_offer` is called by the player.** Cross-reference the emitted `trial_offer_logged` event and the progress contract's state against this document. If the live contract diverges from the docs, the live contract wins — file an issue to update the docs, but code to the live behavior.

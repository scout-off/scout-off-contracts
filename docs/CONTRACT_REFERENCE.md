# Contract Reference

Quick reference for all four ScoutChain Soroban contracts.

---

## registration

Handles player and scout on-chain identity.

| Function | Auth | Description |
|----------|------|-------------|
| `initialize(admin)` | admin | One-time setup |
| `register_player(wallet, vitals, ipfs_hashes)` | wallet | Create player profile at Level 0 |
| `update_profile(player_id, ipfs_hashes)` | player wallet | Update IPFS content hashes |
| `register_scout(wallet, region)` | wallet | Create scout profile |
| `get_player(player_id)` | — | Read player profile |
| `get_player_by_wallet(wallet)` | — | Lookup player by wallet |
| `get_scout(scout_id)` | — | Read scout profile |
| `get_player_count()` | — | Total registered players |
| `get_scout_count()` | — | Total registered scouts |
| `pause_contract()` / `unpause_contract()` | admin | Circuit breaker |
| `health()` | — | Returns true if initialized |

### Dual-Role Wallet Policy

A single wallet address **may register as both a player and a scout**. This is intentional and allowed. A wallet can hold both roles simultaneously without restriction. Duplicate prevention is enforced per role (a wallet cannot register twice as a player, and cannot register twice as a scout), but cross-role registration is permitted.

---

## verification

Manages the trusted validator registry and milestone approvals.

| Function | Auth | Description |
|----------|------|-------------|
| `initialize(admin)` | admin | One-time setup |
| `set_progress_contract(progress_contract)` | admin | Wire cross-contract link |
| `register_validator(wallet, credentials)` | admin | Add trusted validator |
| `revoke_validator(wallet)` | admin | Deactivate validator |
| `approve_milestone(validator_wallet, player_id, description, evidence_hash)` | validator | Record milestone (with ledger_sequence for audit) + cross-call progress.advance_level |
| `get_milestone(player_id, index)` | — | Read a specific milestone |
| `get_milestone_count(player_id)` | — | Total milestones for a player |
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

---

## progress

Maintains the tamper-proof four-tier level state machine.

| Function | Auth | Description |
|----------|------|-------------|
| `initialize(admin)` | admin | One-time setup |
| `advance_level(caller, player_id, milestone_ref)` | caller (validator or scout) | Move player up one level |
| `get_level(player_id)` | — | Current progress level; returns `PlayerNotFound` if player is not registered |
| `get_history_count(player_id)` | — | Number of level changes |
| `get_history_entry(player_id, index)` | — | Specific history entry (`ProgressEntry` includes `ledger_sequence: u32` for tamper-proof auditability) |
| `pause_contract()` / `unpause_contract()` | admin | Circuit breaker |
| `health()` | — | Returns true if initialized |

### Events

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `progress_updated` | event_name, updated_by (Address) | player_id (u64), old_level (ProgressLevel), new_level (ProgressLevel) | Emitted when a player advances to a new level; includes both the previous and new level so indexers do not need to infer the transition from history |
| `admin_transferred` | event_name | old_admin (Address), new_admin (Address) | Emitted when admin rights are transferred to a new address |

---

## scout_access

Handles scout subscriptions, pay-to-contact, and trial offer logging.

| Function | Auth | Description |
|----------|------|-------------|
| `initialize(admin, xlm_token, fee_config)` | admin | One-time setup |
| `update_fee_config(fee_config)` | admin | Adjust fee rates |
| `withdraw_fees(to)` | admin | Collect platform revenue |
| `subscribe(scout, tier)` | scout | Purchase Basic/Pro/Elite subscription |
| `pay_to_contact(scout, player_id)` | scout | Pay micro-fee to unlock player contact |
| `log_trial_offer(scout, player_id, details_hash)` | scout (Elite only) | Record trial offer on-chain |
| `get_subscription(scout)` | — | Read subscription record |
| `get_fee_config()` | — | Current fee configuration |
| `get_accumulated_fees()` | — | Platform fees pending withdrawal |
| `has_contacted(scout, player_id)` | — | Boolean contact check |
| `get_trial_offer(player_id, index)` | — | Read a trial offer |
| `get_trial_count(player_id)` | — | Total trial offers for a player |
| `pause_contract()` / `unpause_contract()` | admin | Circuit breaker |
| `health()` | — | Returns true if initialized |

### Events

| Event | Topics | Data | Description |
|-------|--------|------|-------------|
| `scout_subscribed` | event_name, scout_address | (tier: SubscriptionTier, fee_paid: i128) | Emitted when a scout purchases a subscription; includes the tier and the exact fee charged in stroops |
| `player_contacted` | event_name, scout_address | (player_id: u64, fee_paid: i128) | Emitted when a scout pays to unlock a player's contact details; includes the player id and fee charged in stroops |
| `trial_offer_logged` | event_name, scout_address | player_id: u64 | Emitted when an Elite scout records a trial offer on-chain |
| `fees_withdrawn` | event_name, to_address | amount: i128 | Emitted when the admin withdraws accumulated platform fees |
| `fee_config_updated` | event_name | (old_config: FeeConfig, new_config: FeeConfig) | Emitted when the admin updates the fee configuration; includes both the previous and new config for audit |

---

## Progress Levels

| Integer | Enum | Trigger |
|---------|------|---------|
| 0 | `Unverified` | Profile created |
| 1 | `VerifiedIdentity` | Validator approves identity milestone |
| 2 | `PerformanceMilestones` | Validator approves performance milestone |
| 3 | `EliteTier` | Scout logs trial offer |

---

## Events

| Event | Contract | Emitted When |
|-------|----------|-------------|
| `player_registered` | registration | New player profile created |
| `scout_registered` | registration | New scout profile created |
| `profile_updated` | registration | Player updates IPFS content hashes |
| `milestone_approved` | verification | Validator confirms a player achievement |
| `progress_updated` | progress | Player advances to a new level (data: `player_id`, `new_level`, `milestone_ref`) |
| `scout_subscribed` | scout_access | Scout purchases a subscription |
| `player_contacted` | scout_access | Scout pays to unlock player contact |
| `trial_offer_logged` | scout_access | Scout records a trial offer |
| `fees_withdrawn` | scout_access | Admin withdraws accumulated fees |
| `fee_config_updated` | scout_access | Admin updates fee configuration — data: `(old_config: FeeConfig, new_config: FeeConfig)` |

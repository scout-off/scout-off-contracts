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
| `get_scout_by_wallet(wallet)` | — | Lookup scout by wallet |
| `pause_contract()` / `unpause_contract()` | admin | Circuit breaker |
| `health()` | — | Returns true if initialized |

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

---

## progress

Maintains the tamper-proof four-tier level state machine.

| Function | Auth | Description |
|----------|------|-------------|
| `initialize(admin)` | admin | One-time setup |
| `advance_level(caller, player_id, milestone_ref)` | caller (validator or scout) | Move player up one level |
| `get_level(player_id)` | — | Current progress level |
| `get_history_count(player_id)` | — | Number of level changes |
| `get_history_entry(player_id, index)` | — | Specific history entry |
| `pause_contract()` / `unpause_contract()` | admin | Circuit breaker |
| `health()` | — | Returns true if initialized |

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
| `contract_paused` | registration | Admin pauses the contract |
| `contract_unpaused` | registration | Admin unpauses the contract |
| `milestone_approved` | verification | Validator confirms a player achievement |
| `progress_updated` | progress | Player advances to a new level |
| `scout_subscribed` | scout_access | Scout purchases a subscription |
| `player_contacted` | scout_access | Scout pays to unlock player contact |
| `trial_offer_logged` | scout_access | Scout records a trial offer |
| `fees_withdrawn` | scout_access | Admin withdraws accumulated fees |

# ScoutChain Scout Access Bindings

TypeScript client bindings for the ScoutChain scout access contract. For setup, generation, and usage instructions, see the [bindings README](../README.md); for available contract functions, see the [contract reference](../../docs/CONTRACT_REFERENCE.md).

For reference implementations of the two-step scout onboarding flow —
purchasing a subscription and unlocking player contact details — see
[`examples/subscribe.ts`](examples/README.md).

<!-- AUTO-GENERATED FUNCTION LIST BEGIN - DO NOT EDIT MANUALLY -->

## Functions

The following functions are available in this contract. For complete documentation including parameters, return types, authorization requirements, and examples, see [CONTRACT_REFERENCE.md](../../docs/CONTRACT_REFERENCE.md#scout_access).

- `initialize(admin: Address, xlm_token: Address, fee_config: FeeConfig) -> Result<(), ScoutAccessError>` — One-time contract setup. Validates that `xlm_token` points at a deployed token contract by invoking `decimals()` on it, and that all fee fields are positive with `sub_duration_secs` non-zero. The token probe is read-only and side-effect-free; it exists so that a wrong `xlm_token` address (testnet SAC on mainnet, a typo, a plain account, or a non-token contract) is rejected immediately at deploy time rather than surfacing as an opaque failure on the first `subscribe()` call.
- `propose_admin(new_admin: Address) -> Result<(), ScoutAccessError>` — Store or replace a pending admin proposal. The current admin retains all privileges until the proposed address accepts.
- `accept_admin() -> Result<(), ScoutAccessError>` — Finalize the transfer. The stored pending admin must sign, proving control of the address. Acceptance updates the admin and clears the proposal.
- `transfer_admin(new_admin: Address) -> Result<(), ScoutAccessError>` — Deprecated compatibility alias for `propose_admin`. It creates or replaces a proposal and does not immediately change the admin.
- `set_progress_contract(addr: Address) -> Result<(), ScoutAccessError>` — Register the progress contract address so `log_trial_offer` can call `advance_level` cross-contract (admin only). Unlike `verification.set_progress_contract`, this has no first-call-only guard — it can always be re-invoked to re-wire the link.
- `update_progress_contract(addr: Address) -> Result<(), ScoutAccessError>` — Alias for `set_progress_contract`, provided for naming consistency with `verification.update_progress_contract` so the same verb can be used to re-wire the progress contract link across contracts.
- `update_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>` — Adjust subscription and contact fee rates. Same validation rules as `initialize`.
- `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>` — Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect, giving scouts on-chain-enforced advance notice of any increase.
- `activate_fee_config() -> Result<(), ScoutAccessError>` — Activate a pending fee configuration proposal after the 7-day delay has elapsed.
- `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>` — Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect, giving scouts on-chain-enforced advance notice of any increase.
- `activate_fee_config() -> Result<(), ScoutAccessError>` — Activate a pending fee configuration proposal after the 7-day delay has elapsed.
- `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>` — Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect, giving scouts on-chain-enforced advance notice of any increase.
- `activate_fee_config() -> Result<(), ScoutAccessError>` — Activate a pending fee configuration proposal after the 7-day delay has elapsed.
- `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>` — Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect, giving scouts on-chain-enforced advance notice of any increase.
- `activate_fee_config() -> Result<(), ScoutAccessError>` — Activate a pending fee configuration proposal after the 7-day delay has elapsed.
- `withdraw_fees(to: Address) -> Result<i128, ScoutAccessError>` — Transfer all accumulated platform fees to the given address. Returns the amount withdrawn in stroops.
- `refund_subscription(scout: Address, amount: i128) -> Result<(), ScoutAccessError>` — Emergency admin function to return `amount` XLM (stroops) from the contract balance to a scout. Use when a scout is accidentally double-charged (e.g. by the race condition the upgrade timing guard is designed to prevent).
- `subscribe(scout: Address, tier: SubscriptionTier) -> Result<(), ScoutAccessError>` — Purchase a `Basic`, `Pro`, or `Elite` subscription. The XLM fee is transferred from the scout's wallet to the contract atomically. Downgrades to a cheaper tier while a subscription is still active are rejected.
- `set_auto_renew(scout: Address, enabled: bool) -> Result<(), ScoutAccessError>` — Opt a scout wallet in (`true`) or out (`false`) of automatic subscription renewal.
- `get_auto_renew(scout: Address) -> bool` — Returns `true` if the scout has opted in to automatic subscription renewal, `false` otherwise (including for scouts who have never called `set_auto_renew`).
- `renew_if_due(scout: Address) -> Result<(), ScoutAccessError>` — Renew a scout's subscription if auto-renewal is enabled and the subscription is at or near expiry.
- `pay_to_contact(scout: Address, player_id: u64) -> Result<(), ScoutAccessError>` — Pay a micro-fee to unlock a player's contact details. Scout must have an active (non-expired) subscription.
- `batch_contact_players(scout: Address, player_ids: Vec<u64>) -> Result<u32, ScoutAccessError>` — Contact multiple players in a single transaction. The contact fee is charged once per new player; already-contacted players are silently skipped (no charge). The total fee for all new contacts is deducted in a single token transfer. Returns the count of new contacts recorded.
- `log_trial_offer(scout: Address, player_id: u64, details_hash: String) -> Result<u32, ScoutAccessError>` — Record a trial offer on-chain. Scout must hold an active Elite subscription. `details_hash` is an IPFS/Arweave CID of the offer document. Also calls `progress.advance_level` if the progress contract is registered. Returns the trial offer index.
- `expire_trial_offers(limit: u32) -> Result<u32, ScoutAccessError>` — Admin-only sweep of pending trial offers whose escrow has passed `expires_at`. For each expired entry it refunds the escrowed XLM to the originating scout, removes the `TrialEscrow` record, and emits `trial_offer_expired` — the same cleanup `confirm_trial_offer` performs reactively when called late, run proactively and in bulk. Returns the number of escrows actually swept (`0` if none were due).
- `has_contacted(scout: Address, player_id: u64) -> bool` — Return `true` if the scout has previously called `pay_to_contact` for this player.
- `get_trial_count(player_id: u64) -> u32` — Return the total number of trial offers logged for a player.
- `get_subscription(scout: Address) -> Result<Subscription, ScoutAccessError>` — Read a scout's current subscription record including tier and expiry timestamp.
- `get_fee_config() -> FeeConfig` — Return the current fee configuration.
- `get_fee_config_history() -> Vec<FeeConfigHistoryEntry>` — Return the bounded on-chain history of the last (up to 5) `FeeConfig` values, **oldest-first**.
- `get_accumulated_fees() -> i128` — Return total platform fees pending admin withdrawal (in stroops).
- `get_trial_offer(player_id: u64, index: u32) -> Result<TrialOffer, ScoutAccessError>` — Read a specific trial offer. Indices start at `1`.
- `pause_contract() -> Result<(), ScoutAccessError>` — Halt all state-changing operations.
- `unpause_contract() -> Result<(), ScoutAccessError>` — Resume normal operations after a pause.
- `health() -> ContractHealth` — Return the contract's initialization and pause status.
- `upgrade(new_wasm_hash: BytesN<32>) -> Result<(), ScoutAccessError>` — Replace the contract WASM in-place. Persistent storage (admin, subscriptions, trial offers) survives the upgrade. Admin only.
- `get_scout_contacts(scout: Address) -> Vec<u64>` — Return all player IDs contacted by a scout as an O(1) index lookup (backed by `ScoutContacts` persistent storage key).
- `get_all_trial_offers(player_id: u64) -> Vec<TrialOffer>` — Return all trial offers for a player in a single call. Bounded at 20 to prevent gas exhaustion. Returns an empty `Vec` when no offers exist.
- `get_subscribers_by_tier(tier: SubscriptionTier) -> Vec<Address>` — Return all scout addresses currently subscribed at `tier` (an O(1) index lookup backed by the `TierSubscribers` persistent storage key). Includes expired subscriptions that have not yet been superseded by a renewal or downgrade.
- `get_expiring_subscriptions(before_timestamp: u64, limit: u32) -> Vec<Subscription>` — Return subscriptions whose `expires_at` is at or before `before_timestamp`. This query uses a day-granularity expiry bucket index to avoid scanning every subscription, and it filters renewals by re-checking the live stored `Subscription.expires_at`.
- `has_evidence_access(player_id: u64, scout: Address) -> bool` — Return `true` if `scout` currently holds a non-revoked `EvidenceAccessGrant` for `player_id`.
- `get_evidence_access_grant(player_id: u64, scout: Address) -> Option<EvidenceAccessGrant>` — Return the full grant record for `(player_id, scout)`, if one has ever been issued — including a revoked grant.
- `get_player_access_grants(player_id: u64, offset: u32, limit: u32) -> Vec<EvidenceAccessGrant>` — Page through every `EvidenceAccessGrant` ever issued for `player_id`, oldest-first. `limit` is capped at 50, matching the on-chain index page size.
- `admin_revoke_evidence_access(player_id: u64, scout: Address) -> Result<(), ScoutAccessError>` — Compliance/abuse takedown: mark an `EvidenceAccessGrant` revoked. Does not delete the record. Idempotent.
- `get_contact_record(scout: Address, player_id: u64) -> Option<ContactRecord>` — Return the full `ContactRecord` for a `(scout, player_id)` pair, or `None` if the scout has never contacted this player.
- `get_player_contacts(player_id: u64) -> Vec<Address>` — Return all scout addresses that have contacted a player, as an O(1) index lookup (backed by the `PlayerContacts` persistent storage key).
- `get_player_trial_offers(player_id: u64) -> Vec<TrialOffer>` — Return every trial offer logged for a player, reading the full range from the player's `TrialCounter`. Unlike `get_all_trial_offers`, this is not capped at 20 entries.
- `get_scout_trial_offers(scout: Address) -> Vec<(u64, u32)>` — Return every `(player_id, trial_offer_index)` pair a scout has logged, as an O(1) index lookup (backed by the `ScoutTrialOffers` persistent storage key).
- `version() -> String` — Return the deployed contract version string (from `Cargo.toml` at build time).

<!-- AUTO-GENERATED FUNCTION LIST END -->

#![cfg_attr(target_family = "wasm", no_std)]
mod errors;
mod events;
mod types;

use types::{FeeConfigProposal, ProContactPeriod, ScoutAccessWiringState};

pub use errors::ScoutAccessError;
pub use types::{
    ContactRecord, DataKey, EvidenceAccessGrant, FeeConfig, FeeConfigHistoryEntry, ScoutContactsPage,
    Subscription, SubscriptionTier, TrialEscrow, TrialOffer,
};

use soroban_sdk::{contract, contractimpl, token, Address, Env, String, Vec};

use scoutchain_shared_types::{
    read_wiring_link, require_admin,
    safe_math::{safe_add_i128, safe_add_u32, safe_add_u64, safe_mul_i128},
    validate_cid, write_wiring_link, ContractHealth,
};

// Generated client for cross-contract calls to the progress contract.
// The #[contractclient] macro generates a real Client that performs the
// on-chain call — replacing the hand-written mock that was here before.
mod progress_contract {
    use scoutchain_shared_types::ProgressLevel;
    use soroban_sdk::{contractclient, contracterror, Address, Env};

    #[contracterror]
    #[derive(Copy, Clone, Debug, PartialEq)]
    #[repr(u32)]
    pub enum Error {
        AlreadyAtMaxLevel = 6,
    }

    #[contractclient(name = "Client")]
    #[allow(dead_code)]
    pub trait ProgressContractClient {
        fn advance_level(
            env: Env,
            caller: Address,
            player_id: u64,
            milestone_ref: u32,
        ) -> Result<ProgressLevel, Error>;
    }
}

// Cross-contract client for registration contract, used to verify scout identities
mod registration_contract {
    use soroban_sdk::{contractclient, contracterror, contracttype, Address, Env, String};

    #[contracttype]
    #[derive(Clone, Debug)]
    pub struct ScoutProfile {
        pub scout_id: u64,
        pub wallet: Address,
        pub region: String,
        pub verified: bool,
        pub verification: ScoutVerificationRecord,
        pub registered_at: u64,
    }

    #[contracttype]
    #[derive(Clone, Debug)]
    pub struct ScoutVerificationRecord {
        pub verified: bool,
        pub verified_by: Option<Address>,
        pub verified_at: Option<u64>,
        pub evidence_ref: Option<String>,
        pub method: Option<String>,
    }

    #[contracterror]
    #[derive(Copy, Clone, Debug, PartialEq)]
    #[repr(u32)]
    pub enum Error {
        ScoutNotFound = 12,
    }

    #[contractclient(name = "Client")]
    #[allow(dead_code)]
    pub trait RegistrationContractClient {
        fn get_scout_by_wallet(env: Env, wallet: Address) -> Result<ScoutProfile, Error>;
    }
}

// Instance TTL bump
const INSTANCE_TTL_MIN: u32 = 100;
const INSTANCE_TTL_MAX: u32 = 500;

// Core identity TTL: 30 days at ~5s/ledger ≈ 518_400 ledgers.
// Scout subscriptions and contact records are core identity data.
// Trial offers follow a distinct policy (see TRIAL_* constants below).
const PERSISTENT_TTL_MIN: u32 = 200;
const PERSISTENT_TTL_MAX: u32 = 518_400;

// Admin key bumped to match other contracts, ensuring consistent cross-contract behavior.
const ADMIN_BUMP_LEDGERS: u32 = 518_400;

// Trial offer TTL: ~30 days at 5 s/ledger. Trial offers are ephemeral and do not
// carry the same lifetime significance as identity records, so they follow their own (longer than default but reasonable) schedule.
const TRIAL_TTL_THRESHOLD: u32 = 259_200;
const TRIAL_TTL_EXTEND_TO: u32 = 518_400;

// #795: upper bound on how many OutstandingTrialEscrows entries
// expire_trial_offers will examine in a single call, so a large backlog
// cannot exceed the CPU-instruction budget (see ci/cpu-cost-budget.md).
const EXPIRE_TRIAL_OFFERS_MAX_LIMIT: u32 = 20;
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Minimum interval (seconds) between subscribe calls for the same scout
// to prevent race conditions / double-charging on rapid upgrades.
const MIN_UPGRADE_INTERVAL_SECS: u64 = 3600;

// #456: Minimum cooldown (seconds) between trial offers from the same scout
// to the same player — enforces one pending offer per (scout, player) per day.
const TRIAL_OFFER_COOLDOWN_SECS: u64 = 86_400; // 24 hours

// Minimum fee floors (stroops) to prevent admin from setting negligible fees
// that would effectively remove the monetization model.
const MIN_CONTACT_FEE_STROOPS: i128 = 100_000; // 0.01 XLM
const MIN_SUB_FEE_STROOPS: i128 = 1_000_000; // 0.1 XLM

// Fee config proposal activation delay: 7 days (604,800 seconds) at average
// 5s/ledger ≈ 120,960 ledgers. Scouts have one full week to react to a
// proposed fee increase before it takes effect.
const FEE_CONFIG_PROPOSAL_DELAY_SECS: u64 = 7 * 24 * 60 * 60; // 604,800 seconds

// #826: Bounded on-chain fee config history. 5 entries is enough to cover the
// immediate past plus the pending proposal window (7 days), while keeping the
// storage footprint fixed and predictable regardless of how many times fees
// are updated over the contract's lifetime.
const FEE_CONFIG_HISTORY_CAP: u32 = 5;

// #1040: EvidenceAccessGrant enumeration is paged in fixed-size shards keyed
// by (player_id, page_index) rather than one growing Vec per player, so a
// popular player who has accumulated thousands of grants over time doesn't
// make `get_player_access_grants` (or the write path that appends to it)
// cost proportional to their total history. `MAX_ACCESS_GRANT_PAGE_LIMIT`
// caps `get_player_access_grants`'s `limit` argument at exactly one page, so
// a single call touches at most two pages (the tail of one, the head of the
// next) regardless of `offset` or total grant count. See ci/cpu-cost-budget.md.
const ACCESS_GRANT_PAGE_SIZE: u32 = 50;
const MAX_ACCESS_GRANT_PAGE_LIMIT: u32 = 50;

#[contract]
pub struct ScoutAccessContract;

#[contractimpl]
impl ScoutAccessContract {
    #[inline(always)]
    fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
    }

    // -------------------------------------------------------------------------
    // Admin
    // -------------------------------------------------------------------------

    pub fn initialize(
        env: Env,
        admin: Address,
        xlm_token: Address,
        fee_config: FeeConfig,
    ) -> Result<(), ScoutAccessError> {
        if env.storage().instance().has(&DataKey::Initialized) {
            return Err(ScoutAccessError::AlreadyInitialized);
        }
        // Probe the supplied xlm_token address to confirm it is a deployed
        // token contract before we accept it. A wrong address (testnet SAC on
        // mainnet, a typo, a plain account, a non-token contract) would
        // otherwise only surface as an opaque failure on the first
        // subscribe() call's transfer. The probe is read-only and
        // side-effect-free.
        match token::Client::new(&env, &xlm_token).try_decimals() {
            Ok(_) => {}
            Err(_) => return Err(ScoutAccessError::InvalidInput),
        }
        admin.require_auth();
        Self::validate_fee_config(&fee_config)?;
        Self::bump_instance_ttl(&env);
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ADMIN_BUMP_LEDGERS,
            ADMIN_BUMP_LEDGERS,
        );
        env.storage().instance().set(&DataKey::XlmToken, &xlm_token);
        env.storage()
            .instance()
            .set(&DataKey::FeeConfig, &fee_config);
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .instance()
            .set(&DataKey::AccumulatedFees, &0i128);
        events::contract_initialized(&env, &admin);
        Ok(())
    }

    pub fn update_fee_config(env: Env, fee_config: FeeConfig) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::validate_fee_config(&fee_config)?;

        let old_config = Self::fee_config(&env);

        // Append the outgoing config to the bounded on-chain history (oldest-first,
        // capped at FEE_CONFIG_HISTORY_CAP). When the cap is reached the oldest
        // entry is evicted so the list never grows beyond the fixed capacity.
        let history_key = DataKey::FeeConfigHistory;
        let mut history: Vec<FeeConfigHistoryEntry> = env
            .storage()
            .instance()
            .get(&history_key)
            .unwrap_or_else(|| Vec::new(&env));
        if history.len() >= FEE_CONFIG_HISTORY_CAP {
            history.remove(0);
        }
        history.push_back(FeeConfigHistoryEntry {
            config: old_config.clone(),
            updated_at: env.ledger().timestamp(),
        });
        env.storage().instance().set(&history_key, &history);

        env.storage()
            .instance()
            .set(&DataKey::FeeConfig, &fee_config);

        events::fee_config_updated(&env, &admin, &old_config, &fee_config);
        // `update_fee_config` is the atomic, immediate, no-delay path — flag it
        // as such so indexers/auditors can distinguish it from a delay-respecting
        // `activate_fee_config` call, both of which otherwise emit an identical
        // `fee_config_updated` event. See docs/FEE_CONFIG_PROPOSAL_DESIGN.md.
        events::fee_config_delay_bypassed(&env, &admin, &old_config, &fee_config);
        Ok(())
    }

    /// Propose a new fee configuration. If all fees are ≤ current fees (decreases only),
    /// the config is immediately activated. Otherwise, it is stored as pending and requires
    /// `activate_fee_config` after a 7-day delay to take effect.
    pub fn propose_fee_config(env: Env, fee_config: FeeConfig) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::validate_fee_config(&fee_config)?;

        // Check if a pending proposal already exists
        if env.storage().persistent().has(&DataKey::PendingFeeConfig) {
            return Err(ScoutAccessError::PendingFeeConfigAlreadyExists);
        }

        let current_config = Self::fee_config(&env);
        let now = env.ledger().timestamp();

        // Check if this is a pure decrease (all fees lower or equal)
        let is_decrease_or_no_change = fee_config.contact_fee_stroops
            <= current_config.contact_fee_stroops
            && fee_config.basic_sub_stroops <= current_config.basic_sub_stroops
            && fee_config.pro_sub_stroops <= current_config.pro_sub_stroops
            && fee_config.elite_sub_stroops <= current_config.elite_sub_stroops;

        if is_decrease_or_no_change {
            // Immediate activation for decreases
            env.storage()
                .instance()
                .set(&DataKey::FeeConfig, &fee_config);

            // Emit both events in the same transaction
            events::fee_config_proposed(&env, &admin, &fee_config, now);
            events::fee_config_updated(&env, &admin, &current_config, &fee_config);
        } else {
            // Store as pending for increases
            let proposal = FeeConfigProposal {
                config: fee_config.clone(),
                proposed_at: now,
            };
            env.storage()
                .persistent()
                .set(&DataKey::PendingFeeConfig, &proposal);
            env.storage().persistent().extend_ttl(
                &DataKey::PendingFeeConfig,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );

            events::fee_config_proposed(&env, &admin, &fee_config, now);
        }

        Ok(())
    }

    /// Activate a pending fee configuration proposal after the 7-day delay has elapsed.
    pub fn activate_fee_config(env: Env) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        // Retrieve the pending proposal
        let proposal: FeeConfigProposal = env
            .storage()
            .persistent()
            .get(&DataKey::PendingFeeConfig)
            .ok_or(ScoutAccessError::NoPendingFeeConfig)?;

        let now = env.ledger().timestamp();
        let activation_time = proposal
            .proposed_at
            .checked_add(FEE_CONFIG_PROPOSAL_DELAY_SECS)
            .ok_or(ScoutAccessError::Overflow)?;

        // Check that the delay has elapsed
        if now < activation_time {
            return Err(ScoutAccessError::FeeConfigProposalNotReady);
        }

        // Get the currently active config for the event
        let old_config = Self::fee_config(&env);

        // Move pending to active
        env.storage()
            .instance()
            .set(&DataKey::FeeConfig, &proposal.config);

        // Clear pending state
        env.storage()
            .persistent()
            .remove(&DataKey::PendingFeeConfig);

        events::fee_config_updated(&env, &admin, &old_config, &proposal.config);
        Ok(())
    }

    pub fn withdraw_fees(env: Env, to: Address) -> Result<i128, ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let key = DataKey::AccumulatedFees;
        let fees: i128 = env.storage().instance().get(&key).unwrap_or(0i128);
        if fees == 0 {
            return Err(ScoutAccessError::NoFeesToWithdraw);
        }
        
        // ISSUE #1138: Ensure withdraw_fees does not reduce balance below EscrowedTotal
        let xlm = Self::get_token(&env)?;
        let contract_addr = env.current_contract_address();
        let balance = token::Client::new(&env, &xlm).balance(&contract_addr);
        let escrow_total_key = DataKey::EscrowedTotal;
        let escrowed: i128 = env.storage().instance().get(&escrow_total_key).unwrap_or(0i128);
        let available = balance.saturating_sub(escrowed);
        let withdrawable = fees.min(available);
        
        if withdrawable <= 0 {
            return Err(ScoutAccessError::NoFeesToWithdraw);
        }
        
        token::Client::new(&env, &xlm).transfer(&contract_addr, &to, &withdrawable);
        env.storage().instance().set(&key, &(fees - withdrawable));
        events::fees_withdrawn(&env, &admin, &to, withdrawable);
        Ok(withdrawable)
    }

    pub fn pause_contract(env: Env) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ScoutAccessError::NotInitialized)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        events::contract_paused(&env, &admin);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ScoutAccessError::NotInitialized)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        events::contract_unpaused(&env, &admin);
        Ok(())
    }

    /// Pause the `pay_to_contact` function independently (function-scoped circuit
    /// breaker), mirroring `verification.pause_approve_milestone`. The
    /// whole-contract pause still takes precedence; this enables granular control
    /// when only fee-charging contact needs to be halted (e.g. a suspected
    /// fee-calculation bug or a griefing attack) without blocking scouts from
    /// reading their subscription status or `subscribe`/trial-offer flows.
    /// All other functions remain operational. Admin only. (issue #1056)
    pub fn pause_pay_to_contact(env: Env) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ScoutAccessError::NotInitialized)?;
        env.storage()
            .instance()
            .set(&DataKey::PausedPayToContact, &true);
        events::pay_to_contact_paused(&env, &admin);
        Ok(())
    }

    /// Unpause the `pay_to_contact` function (function-scoped circuit breaker).
    /// Admin only. (issue #1056)
    pub fn unpause_pay_to_contact(env: Env) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ScoutAccessError::NotInitialized)?;
        env.storage()
            .instance()
            .set(&DataKey::PausedPayToContact, &false);
        events::pay_to_contact_unpaused(&env, &admin);
        Ok(())
    }

    /// Register the progress contract address so log_trial_offer can
    /// atomically advance the player to Level 3 (admin only).
    ///
    /// Unlike `verification.set_progress_contract`, this has no
    /// first-call-only guard: it can always be re-invoked to re-wire the
    /// link.
    pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let epoch = write_wiring_link(
            &env,
            &DataKey::ProgressContract,
            &DataKey::ProgressContractEpoch,
            &addr,
        );
        events::progress_contract_updated(&env, &admin, &addr);
        events::wiring_updated(&env, &admin, "progress_contract", &addr, epoch);
        Ok(())
    }

    /// Alias for `set_progress_contract`, kept for naming consistency with
    /// `verification.update_progress_contract` — operators re-wiring after
    /// the initial deployment can use the same verb across contracts.
    pub fn update_progress_contract(env: Env, addr: Address) -> Result<(), ScoutAccessError> {
        Self::set_progress_contract(env, addr)
    }

    /// Return the configured progress contract address, or `None` if the
    /// link has not been configured. Read-only and requires no auth.
    pub fn get_progress_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::ProgressContract)
    }

    /// Wire the registration contract address for Pro-tier scout verification gating.
    /// Admin only. Can be re-invoked to re-wire the link.
    pub fn set_registration_contract(env: Env, addr: Address) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let epoch = write_wiring_link(
            &env,
            &DataKey::RegistrationContract,
            &DataKey::RegistrationContractEpoch,
            &addr,
        );
        events::registration_contract_updated(&env, &admin, &addr);
        events::wiring_updated(&env, &admin, "registration_contract", &addr, epoch);
        Ok(())
    }

    /// Returns a snapshot of both cross-contract peer address pointers held
    /// by this contract (progress, registration), each with its address and
    /// re-wiring epoch.
    ///
    /// This is a **read-only** function — it does not require auth, does not
    /// modify state, and is intentionally exempt from the pause/init guards
    /// so it remains callable even on a mis-wired contract, matching
    /// `progress.get_wiring_state()`. See `docs/WIRING_REGISTRY_DESIGN.md`.
    pub fn get_wiring_state(env: Env) -> ScoutAccessWiringState {
        let progress_contract = read_wiring_link(
            &env,
            &DataKey::ProgressContract,
            &DataKey::ProgressContractEpoch,
        );
        let registration_contract = read_wiring_link(
            &env,
            &DataKey::RegistrationContract,
            &DataKey::RegistrationContractEpoch,
        );
        ScoutAccessWiringState {
            progress_contract,
            registration_contract,
        }
    }

    /// Emergency refund: admin returns `amount` XLM (stroops) from the
    /// contract balance to `scout`.  Use when a scout is accidentally
    /// double-charged (e.g. by the race condition this interval guard
    /// is designed to prevent).
    pub fn refund_subscription(
        env: Env,
        scout: Address,
        amount: i128,
    ) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        if amount <= 0 {
            return Err(ScoutAccessError::InvalidInput);
        }
        let xlm = Self::get_token(&env)?;
        let contract_addr = env.current_contract_address();
        let balance = token::Client::new(&env, &xlm).balance(&contract_addr);
        if amount > balance {
            return Err(ScoutAccessError::InsufficientFee);
        }
        token::Client::new(&env, &xlm).transfer(&contract_addr, &scout, &amount);
        events::subscription_refunded(&env, &scout, amount);
        Ok(())
    }

    /// Upgrade the contract WASM. Admin auth required.
    /// Persistent storage (including Admin) survives this call.
    pub fn upgrade(
        env: Env,
        new_wasm_hash: soroban_sdk::BytesN<32>,
    ) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Scout subscription
    // -------------------------------------------------------------------------

    /// Purchase a scout subscription.
    ///
    /// Payment flow:
    /// 1. Transfer XLM from scout to contract via `token::Client::transfer`.
    /// 2. Add fee to `AccumulatedFees` in instance storage.
    /// 3. Write `Subscription` record to persistent storage.
    ///
    /// Scout must pre-approve the XLM transfer. Downgrades before expiry are rejected.
    ///
    /// Emits `subscription_created` for a brand-new subscription or
    /// `subscription_renewed` when an existing (possibly active) subscription
    /// is replaced. Both events include scout address, tier, subscribed_at, and
    /// expires_at so off-chain indexers can reconstruct the full subscription
    /// history from events alone (closes #462).
    pub fn subscribe(
        env: Env,
        scout: Address,
        tier: SubscriptionTier,
    ) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        scout.require_auth();

        let now = env.ledger().timestamp();

        // Track whether this is a renewal/upgrade of an existing subscription.
        let is_renewal = env
            .storage()
            .persistent()
            .has(&DataKey::Subscription(scout.clone()));

        // Downgrade guard: if an active subscription exists, only allow same
        // tier or an upgrade. Downgrades before expiry are rejected.
        // Also enforce a minimum interval between subscribe calls to prevent
        // race conditions / double-charging on rapid upgrades.
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, Subscription>(&DataKey::Subscription(scout.clone()))
        {
            if now <= existing.expires_at {
                if Self::tier_rank(&tier) < Self::tier_rank(&existing.tier) {
                    return Err(ScoutAccessError::SubscriptionDowngradeNotAllowed);
                }
                if Self::tier_rank(&tier) > Self::tier_rank(&existing.tier) {
                    let min_next = safe_add_u64(existing.subscribed_at, MIN_UPGRADE_INTERVAL_SECS)
                        .map_err(|_| ScoutAccessError::Overflow)?;
                    if now < min_next {
                        return Err(ScoutAccessError::UpgradeTooSoon);
                    }
                }
            }
        }

        // Sybil resistance: gate Pro-tier subscriptions to verified scouts only.
        // Basic and Elite tiers remain unrestricted.
        if tier == SubscriptionTier::Pro {
            if let Some(reg_contract_addr) = env
                .storage()
                .instance()
                .get::<DataKey, Address>(&DataKey::RegistrationContract)
            {
                let reg_client = registration_contract::Client::new(&env, &reg_contract_addr);
                match reg_client.try_get_scout_by_wallet(&scout) {
                    Ok(Ok(scout_profile)) => {
                        if !scout_profile.verification.verified {
                            return Err(ScoutAccessError::ScoutNotVerified);
                        }
                    }
                    _ => {
                        // Scout not found in registration contract; deny Pro-tier access
                        return Err(ScoutAccessError::ScoutNotVerified);
                    }
                }
            }
            // If registration contract is not wired, allow Pro-tier subscription (graceful degradation)
        }

        let config = Self::fee_config(&env);
        let fee = match &tier {
            SubscriptionTier::Basic => config.basic_sub_stroops,
            SubscriptionTier::Pro => config.pro_sub_stroops,
            SubscriptionTier::Elite => config.elite_sub_stroops,
        };

        Self::collect_fee(&env, &scout, fee)?;

        let expires_at =
            safe_add_u64(now, config.sub_duration_secs).map_err(|_| ScoutAccessError::Overflow)?;

        let sub = Subscription {
            scout: scout.clone(),
            tier: tier.clone(),
            expires_at,
            subscribed_at: now,
        };

        // Remove scout from old tier index if upgrading
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, Subscription>(&DataKey::Subscription(scout.clone()))
        {
            Self::remove_from_tier_index(&env, &scout, &existing.tier);
            // Remove from old expiry bucket so the prior expires_at entry
            // doesn't linger and produce false positives in renewal-reminder queries.
            Self::remove_from_expiry_bucket(&env, &scout, existing.expires_at);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Subscription(scout.clone()), &sub);
        env.storage().persistent().extend_ttl(
            &DataKey::Subscription(scout.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // ISSUE #1139: Add scout to tier index for get_subscribers_by_tier
        Self::add_to_tier_index(&env, &scout, &tier);

        // Add scout to the day-granularity expiry bucket so
        // get_expiring_subscriptions can page through soon-to-expire
        // subscriptions without walking every scout.
        Self::add_to_expiry_bucket(&env, &scout, expires_at);

        // Emit a rich auditable event (closes #462).
        // subscription_renewed covers same-tier renewals and tier upgrades;
        // subscription_created covers a scout's very first subscription.
        if is_renewal {
            events::subscription_renewed(&env, &scout, &tier, now, expires_at);
        } else {
            events::subscription_created(&env, &scout, &tier, now, expires_at);
        }
        // Keep the legacy scout_subscribed event for backward compatibility.
        events::scout_subscribed(&env, &scout, &tier, fee);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Auto-renewal
    // -------------------------------------------------------------------------

    /// Opt a scout wallet in or out of automatic subscription renewal.
    ///
    /// ## Auth model
    ///
    /// Requires the scout's own signature (`scout.require_auth()`).  This
    /// prevents a third party from silently toggling auto-renewal on behalf
    /// of a scout, which would create an unexpected recurring charge.
    ///
    /// ## Usage
    ///
    /// Once enabled, a keeper (off-chain cron job or bot) can call
    /// `renew_if_due` when the scout's subscription is at or near expiry.
    /// The scout must sign the `renew_if_due` transaction itself — the
    /// keeper bot cannot pull funds without a fresh scout signature because
    /// Soroban's token transfer always requires the sender's auth in the
    /// same transaction.  See `renew_if_due` for the auth-model rationale.
    pub fn set_auto_renew(env: Env, scout: Address, enabled: bool) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        scout.require_auth();

        env.storage()
            .persistent()
            .set(&DataKey::AutoRenew(scout.clone()), &enabled);
        env.storage().persistent().extend_ttl(
            &DataKey::AutoRenew(scout.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        events::auto_renew_set(&env, &scout, enabled);
        Ok(())
    }

    /// Return whether a scout has opted in to automatic subscription renewal.
    pub fn get_auto_renew(env: Env, scout: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::AutoRenew(scout))
            .unwrap_or(false)
    }

    /// Renew a scout's subscription if auto-renewal is enabled and the
    /// subscription is at or near expiry.
    ///
    /// ## Auth model — why the scout must sign
    ///
    /// Soroban's token transfer (`token::Client::transfer`) always requires an
    /// authorisation entry for the sender *in the same transaction*.  A third-
    /// party keeper bot calling this function cannot pull XLM from the scout's
    /// wallet on its own: the scout's keypair must sign the transaction that
    /// calls `renew_if_due`, exactly as it does for `subscribe`.
    ///
    /// This is not a limitation — it is the correct security posture.
    /// Auto-renewal does not mean "anyone can charge me"; it means "I (the
    /// scout) authorise this function to renew my subscription for me, and I
    /// am still the one signing the transaction."  A keeper bot's role is to
    /// *prompt* the scout to sign a renewal transaction before expiry, not to
    /// sign on the scout's behalf.
    ///
    /// An allowance/pre-signed-transaction pattern (e.g. Soroban auth trees)
    /// could allow truly permissionless renewal in a future version, but
    /// would require the scout to pre-authorise a specific amount and time
    /// window via `token::Client::approve`, which adds complexity and is
    /// not worth the tradeoff for the current use case.
    ///
    /// ## When a renewal fires
    ///
    /// The renewal fires when **both** conditions hold:
    /// 1. `auto_renew` is `true` for this scout.
    /// 2. The current ledger timestamp is within `sub_duration_secs / 10`
    ///    seconds of expiry (i.e. in the last 10 % of the subscription
    ///    window), **or** the subscription has already expired.
    ///
    /// If neither condition is met the function returns `AutoRenewNotEnabled`
    /// (condition 1 failed) or exits successfully without charging (condition
    /// 2 not yet reached), making it safe for a keeper to call repeatedly
    /// without triggering premature renewals.
    ///
    /// ## Charging
    ///
    /// Uses the same `collect_fee` path as `subscribe`: transfers the
    /// tier-appropriate fee from the scout's wallet to the contract and
    /// accumulates it in `AccumulatedFees`.  All downgrade-guard and
    /// overflow protections from `subscribe` apply identically.
    pub fn renew_if_due(env: Env, scout: Address) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        // The scout must sign this transaction. See auth-model note in the
        // doc comment above — the token transfer requires the scout's auth in
        // the same invocation, so this require_auth is both necessary and
        // sufficient to protect against unauthorised charges.
        scout.require_auth();

        // Check auto-renewal opt-in.
        let auto_renew_enabled: bool = env
            .storage()
            .persistent()
            .get(&DataKey::AutoRenew(scout.clone()))
            .unwrap_or(false);
        if !auto_renew_enabled {
            return Err(ScoutAccessError::AutoRenewNotEnabled);
        }

        // Fetch the existing subscription.  A scout with no subscription cannot
        // be auto-renewed — they must use `subscribe` to create one first.
        let existing: Subscription = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(scout.clone()))
            .ok_or(ScoutAccessError::ScoutNotSubscribed)?;

        let now = env.ledger().timestamp();
        let config = Self::fee_config(&env);

        // Renewal window: last 10 % of the subscription duration, or already
        // expired.  This prevents renewing too early (e.g. the day after
        // subscribing) while still giving the keeper a window before hard expiry.
        let renewal_window_secs = config.sub_duration_secs.checked_div(10).unwrap_or(0).max(1);
        let renewal_due_at = existing.expires_at.saturating_sub(renewal_window_secs);

        if now < renewal_due_at {
            // Not yet in the renewal window — no-op, not an error.
            return Ok(());
        }

        let fee = match &existing.tier {
            SubscriptionTier::Basic => config.basic_sub_stroops,
            SubscriptionTier::Pro => config.pro_sub_stroops,
            SubscriptionTier::Elite => config.elite_sub_stroops,
        };

        Self::collect_fee(&env, &scout, fee)?;

        let expires_at = now
            .checked_add(config.sub_duration_secs)
            .ok_or(ScoutAccessError::Overflow)?;

        let renewed_sub = Subscription {
            scout: scout.clone(),
            tier: existing.tier.clone(),
            expires_at,
            subscribed_at: now,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Subscription(scout.clone()), &renewed_sub);
        env.storage().persistent().extend_ttl(
            &DataKey::Subscription(scout.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // ISSUE #1139: Maintain tier index on renewal
        Self::add_to_tier_index(&env, &scout, &existing.tier);

        events::subscription_auto_renewed(&env, &scout, &existing.tier, now, expires_at);
        // Also emit the legacy scout_subscribed event for backward compatibility.
        events::scout_subscribed(&env, &scout, &existing.tier, fee);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Pay-to-contact
    // -------------------------------------------------------------------------

    /// Helper: resolve the effective Pro-tier contact limit for a scout.
    ///
    /// If the scout has a registered region (from the registration contract) and
    /// a per-region override has been set for that region, return the override.
    /// Otherwise fall back to the platform-wide `FeeConfig.pro_contact_limit`.
    fn effective_pro_contact_limit(env: &Env, scout: &Address) -> u32 {
        let config = Self::fee_config(env);
        let platform_default = config.pro_contact_limit;

        // Attempt to look up the scout's region from the registration contract.
        let reg_contract_addr = match env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::RegistrationContract)
        {
            Some(addr) => addr,
            None => return platform_default,
        };

        let reg_client = registration_contract::Client::new(env, &reg_contract_addr);
        let scout_profile = match reg_client.try_get_scout_by_wallet(scout) {
            Ok(profile) => profile,
            Err(_) => return platform_default,
        };

        // Check for a regional override.
        if let Some(limit) = env
            .storage()
            .persistent()
            .get::<DataKey, u32>(&DataKey::RegionalContactLimit(scout_profile.region))
        {
            return limit;
        }

        platform_default
    }

    /// Helper: check Pro tier contact quota with a specific count (batch support).
    fn check_pro_contact_quota_with_count(
        env: &Env,
        scout: &Address,
        n: u32,
    ) -> Result<(), ScoutAccessError> {
        let sub: Subscription = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(scout.clone()))
            .ok_or(ScoutAccessError::ScoutNotSubscribed)?;

        // Only Pro tier has a per-period quota.
        if sub.tier != SubscriptionTier::Pro {
            return Ok(());
        }

        // Use the same subscription-period counter as `pay_to_contact`.
        // The legacy wall-clock `ContactCount` bucket is retained as an
        // analytics/indexing mirror, but must not be used for enforcement: a
        // subscription can begin in a different calendar month from the
        // previous contact and both entrypoints must share one quota.
        let period_key = DataKey::ProContactCount(scout.clone());
        let period: ProContactPeriod =
            env.storage()
                .persistent()
                .get(&period_key)
                .unwrap_or(ProContactPeriod {
                    period_start: sub.subscribed_at,
                    count: 0,
                });
        let current = if period.period_start == sub.subscribed_at {
            period.count
        } else {
            0u32
        };

        let limit = Self::effective_pro_contact_limit(env, scout);

        if current.saturating_add(requested) > limit {
            return Err(ScoutAccessError::ProContactLimitReached);
        }

        let new_period = ProContactPeriod {
            period_start: sub.subscribed_at,
            count: current_count
                .checked_add(n)
                .ok_or(ScoutAccessError::Overflow)?,
        };
        env.storage().persistent().set(&period_key, &new_period);
        env.storage().persistent().extend_ttl(
            &period_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        Ok(())
    }

    /// Write an `EvidenceAccessGrant(player_id, scout)` and append `scout` to
    /// that player's paged enumeration index, atomically with the caller's
    /// storage writes (both happen inside the same contract invocation, so a
    /// later error in the caller rolls this back along with everything else).
    ///
    /// Called from `pay_to_contact` and `batch_contact_players` — the two
    /// entrypoints that record a *new* `ContactRecord` — exactly once per
    /// newly-recorded contact, so a grant is issued if and only if the
    /// contact fee was actually collected. See `docs/EVIDENCE_PRIVACY.md`.
    fn grant_evidence_access(
        env: &Env,
        player_id: u64,
        scout: &Address,
        tier: &SubscriptionTier,
        granted_at: u64,
    ) -> Result<(), ScoutAccessError> {
        let count_key = DataKey::EvidenceAccessGrantCount(player_id);
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0u32);
        let page_idx = count / ACCESS_GRANT_PAGE_SIZE;

        let page_key = DataKey::EvidenceAccessGrantPage(player_id, page_idx);
        let mut page: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&page_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(env));
        page.push_back(scout.clone());
        env.storage().persistent().set(&page_key, &page);
        env.storage()
            .persistent()
            .extend_ttl(&page_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        let new_count = safe_add_u32(count, 1).map_err(|_| ScoutAccessError::Overflow)?;
        env.storage().persistent().set(&count_key, &new_count);
        env.storage()
            .persistent()
            .extend_ttl(&count_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        let grant_key = DataKey::EvidenceAccessGrant(player_id, scout.clone());
        let grant = EvidenceAccessGrant {
            player_id,
            scout: scout.clone(),
            granted_at,
            tier_at_grant: tier.clone(),
            revoked: false,
            revoked_at: None,
        };
        env.storage().persistent().set(&grant_key, &grant);
        env.storage()
            .persistent()
            .extend_ttl(&grant_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        events::evidence_access_granted(env, player_id, scout, tier);
        Ok(())
    }

    /// Pay a micro-fee to unlock a player's contact details.
    ///
    /// Payment flow:
    /// 1. Transfer `contact_fee_stroops` XLM from scout to contract via `token::Client::transfer`.
    /// 2. Add fee to `AccumulatedFees` in instance storage.
    /// 3. Write contact record to persistent storage (prevents duplicate contacts).
    ///
    /// Scout must have an active, non-expired subscription.
    /// Pro tier scouts are limited to `pro_contact_limit` contacts per month.
    pub fn pay_to_contact(
        env: Env,
        scout: Address,
        player_id: u64,
    ) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        Self::require_pay_to_contact_not_paused(&env)?;
        scout.require_auth();

        let subscription: Subscription = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(scout.clone()))
            .ok_or(ScoutAccessError::ScoutNotSubscribed)?;
        if subscription.expires_at < env.ledger().timestamp() {
            return Err(ScoutAccessError::SubscriptionExpired);
        }

        let contact_key = DataKey::ContactRecord(player_id, scout.clone());
        if env.storage().persistent().has(&contact_key) {
            return Err(ScoutAccessError::AlreadyContacted);
        }

        let config = Self::fee_config(&env);

        // Pro-tier quota enforcement: limit contacts to pro_contact_limit per
        // subscription period.  The counter resets automatically on renewal
        // because a new period_start is stored when the scout subscribes again.
        if subscription.tier == SubscriptionTier::Pro {
            let period_key = DataKey::ProContactCount(scout.clone());
            let period: ProContactPeriod =
                env.storage()
                    .persistent()
                    .get(&period_key)
                    .unwrap_or(ProContactPeriod {
                        period_start: subscription.subscribed_at,
                        count: 0,
                    });
            // If the stored period_start predates the current subscription,
            // treat the counter as zero (subscription was renewed).
            let current_count = if period.period_start == subscription.subscribed_at {
                period.count
            } else {
                0u32
            };
            if current_count >= Self::effective_pro_contact_limit(&env, &scout) {
                return Err(ScoutAccessError::ProContactLimitReached);
            }
            let new_period = ProContactPeriod {
                period_start: subscription.subscribed_at,
                count: safe_add_u32(current_count, 1).map_err(|_| ScoutAccessError::Overflow)?,
            };
            env.storage().persistent().set(&period_key, &new_period);
            env.storage().persistent().extend_ttl(
                &period_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
        }

        Self::collect_fee(&env, &scout, config.contact_fee_stroops)?;

        let record = ContactRecord {
            player_id,
            scout: scout.clone(),
            contacted_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&contact_key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&contact_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        env.storage().persistent().extend_ttl(
            &DataKey::Subscription(scout.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Update scout-centric contact index
        let index_key = DataKey::ScoutContacts(scout.clone());
        let mut contacted: soroban_sdk::Vec<u64> = env
            .storage()
            .persistent()
            .get(&index_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if !contacted.contains(player_id) {
            contacted.push_back(player_id);
        }
        env.storage().persistent().set(&index_key, &contacted);
        env.storage()
            .persistent()
            .extend_ttl(&index_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // Update player-centric inbound contact index so a player can list
        // all scouts who have contacted them directly from on-chain state
        // without replaying off-chain events.
        let player_index_key = DataKey::PlayerContacts(player_id);
        let mut inbound: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&player_index_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if !inbound.contains(&scout) {
            inbound.push_back(scout.clone());
        }
        env.storage().persistent().set(&player_index_key, &inbound);
        env.storage().persistent().extend_ttl(
            &player_index_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // #1040: a successful pay_to_contact is the on-chain authorization
        // event for this scout to request the wrapped decryption key for
        // player_id's confidential evidence. This runs after every check
        // above has already passed and the fee has already been collected,
        // so it is unreachable on any rejected call (see
        // adversarial_atomicity.rs / atomic_fee_settlement.rs for the
        // atomicity tests this must satisfy).
        Self::grant_evidence_access(&env, player_id, &scout, &subscription.tier, record.contacted_at)?;

        events::player_contacted(&env, player_id, &scout, config.contact_fee_stroops);
        Ok(())
    }

    /// Contact multiple players in a single transaction. Charges the contact fee
    /// for each player that has not already been contacted. Already-contacted
    /// players are silently skipped (no charge). The total fee for all new contacts
    /// is deducted in a single token transfer. Returns the number of new contacts
    /// that were recorded.
    ///
    /// Scout must have an active (non-expired) subscription.
    /// Pro tier scouts are limited to `pro_contact_limit` contacts per month.
    pub fn batch_contact_players(
        env: Env,
        scout: Address,
        player_ids: soroban_sdk::Vec<u64>,
    ) -> Result<u32, ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        scout.require_auth();
        let sub = Self::require_active_subscription(&env, &scout)?;

        let config = Self::fee_config(&env);
        let mut new_contacts: u32 = 0;

        // First pass: count new (uncharged) contacts to compute total fee.
        // `seen` deduplicates player_ids *within this call* -- storage isn't
        // mutated until the second pass, so without this a repeated
        // player_id in the input would be counted (and charged) more than
        // once even though only a single ContactRecord is ever written.
        let mut seen: soroban_sdk::Vec<u64> = soroban_sdk::Vec::new(&env);
        for i in 0..player_ids.len() {
            let player_id = player_ids.get(i).unwrap();
            if seen.contains(player_id) {
                continue;
            }
            seen.push_back(player_id);
            if !env
                .storage()
                .persistent()
                .has(&DataKey::ContactRecord(player_id, scout.clone()))
            {
                new_contacts =
                    safe_add_u32(new_contacts, 1).map_err(|_| ScoutAccessError::Overflow)?;
            }
        }

        if new_contacts == 0 {
            return Ok(0);
        }

        // Unified Pro-tier quota: check limit and increment atomically for
        // the entire batch in one call. Uses the same renewal-aware logic as
        // pay_to_contact (n=1) via the shared helper.
        Self::check_and_reserve_pro_quota(&env, &scout, new_contacts)?;

        // Single token transfer for all new contacts combined.
        let total_fee = safe_mul_i128(config.contact_fee_stroops, new_contacts as i128)
            .map_err(|_| ScoutAccessError::Overflow)?;
        Self::collect_fee(&env, &scout, total_fee)?;

        // Second pass: write contact records and emit events.
        for i in 0..player_ids.len() {
            let player_id = player_ids.get(i).unwrap();
            let contact_key = DataKey::ContactRecord(player_id, scout.clone());
            if env.storage().persistent().has(&contact_key) {
                continue;
            }
            let record = ContactRecord {
                player_id,
                scout: scout.clone(),
                contacted_at: env.ledger().timestamp(),
            };
            env.storage().persistent().set(&contact_key, &record);
            env.storage().persistent().extend_ttl(
                &contact_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );

            // Update scout-centric outbound index
            let scout_index_key = DataKey::ScoutContacts(scout.clone());
            let mut scout_contacted: soroban_sdk::Vec<u64> = env
                .storage()
                .persistent()
                .get(&scout_index_key)
                .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
            if !scout_contacted.contains(player_id) {
                scout_contacted.push_back(player_id);
            }
            env.storage()
                .persistent()
                .set(&scout_index_key, &scout_contacted);
            env.storage().persistent().extend_ttl(
                &scout_index_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );

            // Update player-centric inbound index
            let player_index_key = DataKey::PlayerContacts(player_id);
            let mut inbound: soroban_sdk::Vec<Address> = env
                .storage()
                .persistent()
                .get(&player_index_key)
                .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
            if !inbound.contains(&scout) {
                inbound.push_back(scout.clone());
            }
            env.storage().persistent().set(&player_index_key, &inbound);
            env.storage().persistent().extend_ttl(
                &player_index_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );

            // #1040: same grant issuance as pay_to_contact, for each new
            // contact this batch call actually recorded (and charged) — a
            // scout must not be able to bypass evidence-access grants by
            // reaching a contact through the batch entrypoint instead.
            Self::grant_evidence_access(&env, player_id, &scout, &sub.tier, record.contacted_at)?;

            events::player_contacted(&env, player_id, &scout, config.contact_fee_stroops);
        }

        Self::increment_contact_count_by(&env, &scout, new_contacts);

        // Keep the renewal-aware Pro quota counter in lockstep with the
        // legacy monthly analytics counter. `pay_to_contact` writes this
        // counter before charging; batch writes it after the single charge.
        if sub.tier == SubscriptionTier::Pro {
            let period_key = DataKey::ProContactCount(scout.clone());
            let period: ProContactPeriod =
                env.storage()
                    .persistent()
                    .get(&period_key)
                    .unwrap_or(ProContactPeriod {
                        period_start: sub.subscribed_at,
                        count: 0,
                    });
            let current = if period.period_start == sub.subscribed_at {
                period.count
            } else {
                0u32
            };
            let updated = ProContactPeriod {
                period_start: sub.subscribed_at,
                count: safe_add_u32(current, new_contacts)
                    .map_err(|_| ScoutAccessError::Overflow)?,
            };
            env.storage().persistent().set(&period_key, &updated);
            env.storage().persistent().extend_ttl(
                &period_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
        }

        env.storage().persistent().extend_ttl(
            &DataKey::Subscription(scout.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        Ok(new_contacts)
    }

    // -------------------------------------------------------------------------
    // Trial offer
    // -------------------------------------------------------------------------

    /// Log a trial offer on-chain. Scout must have an Elite subscription.
    /// Also calls progress.advance_level if the progress contract is registered.
    pub fn log_trial_offer(
        env: Env,
        scout: Address,
        player_id: u64,
        details_hash: String,
    ) -> Result<u32, ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        scout.require_auth();

        validate_cid(&details_hash).map_err(|_| ScoutAccessError::InvalidInput)?;

        let sub = Self::require_active_subscription(&env, &scout)?;
        if sub.tier != SubscriptionTier::Elite {
            return Err(ScoutAccessError::Unauthorized);
        }
        env.storage().persistent().extend_ttl(
            &DataKey::Subscription(scout.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Verify the scout has previously contacted this player.
        let contact_key = DataKey::ContactRecord(player_id, scout.clone());
        if !env.storage().persistent().has(&contact_key) {
            return Err(ScoutAccessError::Unauthorized);
        }
        env.storage()
            .persistent()
            .extend_ttl(&contact_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // #456: Enforce per-(scout, player) cooldown to prevent offer flooding.
        // Reject a second offer from the same scout to the same player within
        // TRIAL_OFFER_COOLDOWN_SECS (24 h). Offers to different players are
        // independent and are not rate-limited against each other.
        let rate_key = DataKey::TrialOfferLastSent(scout.clone(), player_id);
        let now = env.ledger().timestamp();
        if let Some(last_sent) = env.storage().persistent().get::<DataKey, u64>(&rate_key) {
            let next_allowed = safe_add_u64(last_sent, TRIAL_OFFER_COOLDOWN_SECS)
                .map_err(|_| ScoutAccessError::Overflow)?;
            if now < next_allowed {
                return Err(ScoutAccessError::TrialOfferRateLimited);
            }
        }

        let counter_key = DataKey::TrialCounter(player_id);
        let index: u32 = env.storage().persistent().get(&counter_key).unwrap_or(0u32);
        let next_index = safe_add_u32(index, 1).map_err(|_| ScoutAccessError::Overflow)?;

        let offer = TrialOffer {
            player_id,
            scout: scout.clone(),
            details_hash,
            logged_at: now,
        };

        // #455-style ordering: all persistent writes before event emission.
        env.storage()
            .persistent()
            .set(&DataKey::TrialOffer(player_id, next_index), &offer);
        env.storage().persistent().set(&counter_key, &next_index);
        env.storage().persistent().extend_ttl(
            &DataKey::TrialOffer(player_id, next_index),
            TRIAL_TTL_THRESHOLD,
            TRIAL_TTL_EXTEND_TO,
        );
        env.storage().persistent().extend_ttl(
            &counter_key,
            TRIAL_TTL_THRESHOLD,
            TRIAL_TTL_EXTEND_TO,
        );
        // #456: Record the timestamp of this offer for future cooldown checks.
        env.storage().persistent().set(&rate_key, &now);
        env.storage()
            .persistent()
            .extend_ttl(&rate_key, TRIAL_TTL_THRESHOLD, TRIAL_TTL_EXTEND_TO);

        // #468: Update per-scout trial offer index so scouts can enumerate all
        // trial offers they have logged without an off-chain event index.
        let scout_index_key = DataKey::ScoutTrialOffers(scout.clone());
        let mut scout_offers: soroban_sdk::Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&scout_index_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        scout_offers.push_back((player_id, next_index));
        env.storage()
            .persistent()
            .set(&scout_index_key, &scout_offers);
        env.storage().persistent().extend_ttl(
            &scout_index_key,
            TRIAL_TTL_THRESHOLD,
            TRIAL_TTL_EXTEND_TO,
        );

        // Store escrow for the trial offer
        let fee_cfg: FeeConfig = env
            .storage()
            .instance()
            .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
            .ok_or(ScoutAccessError::InvalidInput)?;
        let escrow_amount = fee_cfg.trial_offer_escrow_stroops;
        let expires_at = safe_add_u64(now, fee_cfg.trial_offer_expiry_secs)
            .map_err(|_| ScoutAccessError::Overflow)?;

        // #795: actually collect the escrow — without this transfer the
        // TrialEscrow record was a bookkeeping-only promise, and every
        // refund path (confirm_trial_offer's late-expiry branch and
        // expire_trial_offers) paid out funds the contract never held.
        let token_addr = Self::get_token(&env)?;
        let contract_addr = env.current_contract_address();
        token::Client::new(&env, &token_addr).transfer(&scout, &contract_addr, &escrow_amount);

        // ISSUE #1138: Track escrowed total to segregate escrow-held funds
        // from withdrawable fees. Prevent withdraw_fees from depleting the
        // escrow reserve.
        let escrow_total_key = DataKey::EscrowedTotal;
        let current_escrowed: i128 = env.storage().instance().get(&escrow_total_key).unwrap_or(0i128);
        let new_escrowed = safe_add_i128(current_escrowed, escrow_amount)
            .map_err(|_| ScoutAccessError::Overflow)?;
        env.storage().instance().set(&escrow_total_key, &new_escrowed);

        let escrow = TrialEscrow {
            amount: escrow_amount,
            expires_at,
        };
        env.storage()
            .persistent()
            .set(&DataKey::TrialEscrow(player_id, next_index), &escrow);
        // Extend TrialEscrow TTL so the escrow record outlives its own expiry
        // window. Without this, Soroban assigns the default minimal persistent
        // TTL (~4,096 ledgers ≈ 5.7 hours), which is shorter than the
        // configured trial_offer_expiry_secs and could cause the record to
        // become archived before either confirm_trial_offer or
        // expire_trial_offers resolves it — locking the escrowed XLM with no
        // read-accessible path to the two functions that can release it.
        env.storage().persistent().extend_ttl(
            &DataKey::TrialEscrow(player_id, next_index),
            TRIAL_TTL_THRESHOLD,
            TRIAL_TTL_EXTEND_TO,
        );

        // #795: index this escrow so expire_trial_offers can sweep it later
        // without an off-chain indexer.
        let outstanding_key = DataKey::OutstandingTrialEscrows;
        let mut outstanding: soroban_sdk::Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&outstanding_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        outstanding.push_back((player_id, next_index));
        env.storage()
            .persistent()
            .set(&outstanding_key, &outstanding);
        env.storage().persistent().extend_ttl(
            &outstanding_key,
            TRIAL_TTL_THRESHOLD,
            TRIAL_TTL_EXTEND_TO,
        );
        // Emit logged event (no immediate level advance)
        events::trial_offer_logged(&env, player_id, &scout);
        Ok(next_index)
    }
    /// Confirm a previously logged trial offer. Called by the player (or validator) to release escrow and advance level.
    ///
    /// `idempotency_nonce` is an optional caller-supplied token. If provided and
    /// the nonce has already been processed, the function returns `Ok(())`
    /// without replaying escrow cleanup or level advancement. This makes
    /// retries after `ProgressCallFailed` safe.
    pub fn confirm_trial_offer(
        env: Env,
        player_wallet: Address,
        player_id: u64,
        index: u32,
        idempotency_nonce: Option<String>,
    ) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        // Player must authorize
        player_wallet.require_auth();

        // ISSUE #1136: Cross-contract verify that player_wallet owns player_id
        // in the registration contract before proceeding with escrow release.
        // This prevents any wallet from releasing escrow and advancing an
        // arbitrary player to Level 3.
        let reg_contract_addr = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::RegistrationContract)
            .ok_or(ScoutAccessError::InvalidInput)?; // Fail closed if registration not wired
        let reg_client = registration_contract::Client::new(&env, &reg_contract_addr);
        let player_profile = reg_client
            .get_player(&player_id)
            .map_err(|_| ScoutAccessError::Unauthorized)?;
        if player_profile.owner != player_wallet {
            return Err(ScoutAccessError::Unauthorized);
        }

        // ISSUE #1137: Idempotency nonce must be derived from (player_id, index)
        // to prevent griefers from pre-seeding arbitrary caller-supplied nonces.
        // The nonce key now binds to the specific trial offer rather than an
        // unauthenticated global string.
        let nonce_key = DataKey::ConfirmationNonce(
            String::from_str(&env, &format!("trial_confirm:{}:{}", player_id, index))
        );
        if env.storage().persistent().has(&nonce_key) {
            return Ok(());
        }

        // Load escrow record
        let escrow: TrialEscrow = env
            .storage()
            .persistent()
            .get(&DataKey::TrialEscrow(player_id, index))
            .ok_or(ScoutAccessError::TrialOfferAlreadyConfirmed)?;

        // Load the original offer to get the scout address
        let offer: TrialOffer = env
            .storage()
            .persistent()
            .get(&DataKey::TrialOffer(player_id, index))
            .ok_or(ScoutAccessError::TrialOfferNotFound)?;

        let now = env.ledger().timestamp();
        // Check expiry
        if now > escrow.expires_at {
            // Refund escrow to scout
            let token_addr = Self::get_token(&env)?;
            let contract_addr = env.current_contract_address();
            let balance = token::Client::new(&env, &token_addr).balance(&contract_addr);
            if escrow.amount > balance {
                return Err(ScoutAccessError::InsufficientFee);
            }
            token::Client::new(&env, &token_addr).transfer(
                &contract_addr,
                &offer.scout,
                &escrow.amount,
            );
            // ISSUE #1138: Decrement escrowed total on refund
            let escrow_total_key = DataKey::EscrowedTotal;
            let current_escrowed: i128 = env.storage().instance().get(&escrow_total_key).unwrap_or(0i128);
            let new_escrowed = current_escrowed.saturating_sub(escrow.amount);
            env.storage().instance().set(&escrow_total_key, &new_escrowed);
            // Cleanup escrow
            env.storage()
                .persistent()
                .remove(&DataKey::TrialEscrow(player_id, index));
            Self::remove_from_outstanding_trial_escrows(&env, player_id, index);
            // Emit expiry event. Returning `Ok(())` is intentional: Soroban
            // rolls back all state changes when a contract returns `Err`, so
            // returning `TrialOfferExpired` here would undo the refund and
            // escrow cleanup. The event is the durable outcome signal.
            events::trial_offer_expired(&env, player_id, &offer.scout, index);
            return Ok(());
        }

        // Cross-contract call: advance the player to Level 3 if progress contract is set.
        // Call progress contract to advance level (using the index as milestone reference).
        let progress_addr = match env
            .storage()
            .instance()
            .get::<DataKey, soroban_sdk::Address>(&DataKey::ProgressContract)
        {
            Some(addr) => addr,
            None => {
                // Progress contract not configured — emit diagnostic so the indexer
                // can alert on missing wiring rather than returning an opaque error.
                events::progress_contract_not_set(&env, player_id);
                return Err(ScoutAccessError::InvalidInput);
            }
        };
        let progress_client = progress_contract::Client::new(&env, &progress_addr);
        match progress_client.try_advance_level(&env.current_contract_address(), &player_id, &index)
        {
            Ok(_) => {}
            Err(e) => {
                // Extract numeric error code from the contract error, if any.
                let code = match &e {
                    Ok(pe) => *pe as u32,
                    Err(_) => 0u32,
                };
                events::progress_call_failed(&env, player_id, code);
                return Err(ScoutAccessError::ProgressCallFailed);
            }
        }

        // Persist idempotency nonce after successful level advancement so that
        // a retry can safely detect the offer was already confirmed.
        env.storage().persistent().set(&nonce_key, &());
        env.storage().persistent().extend_ttl(
            &nonce_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // ISSUE #1138: Decrement escrowed total on successful confirmation
        let escrow_total_key = DataKey::EscrowedTotal;
        let current_escrowed: i128 = env.storage().instance().get(&escrow_total_key).unwrap_or(0i128);
        let new_escrowed = current_escrowed.saturating_sub(escrow.amount);
        env.storage().instance().set(&escrow_total_key, &new_escrowed);

        // Cleanup escrow after successful confirmation
        env.storage()
            .persistent()
            .remove(&DataKey::TrialEscrow(player_id, index));
        Self::remove_from_outstanding_trial_escrows(&env, player_id, index);
        // Emit confirmed event
        events::trial_offer_confirmed(&env, player_id, &offer.scout, index);
        Ok(())
    }

    /// Admin helper to sweep pending trial offers that have passed their
    /// expiry window: refunds the escrowed XLM to the originating scout,
    /// removes the `TrialEscrow` record, and emits `trial_offer_expired`
    /// for each swept entry. This is the same cleanup `confirm_trial_offer`
    /// already performs reactively when called late (#795), run proactively
    /// and in bulk.
    ///
    /// `limit` bounds how many `OutstandingTrialEscrows` entries are examined
    /// in this call (capped at `EXPIRE_TRIAL_OFFERS_MAX_LIMIT`), so a large
    /// backlog cannot exceed the CPU-instruction budget in a single
    /// invocation — call repeatedly to drain a larger backlog. Entries not
    /// yet past `expires_at` are left in place for a later call. Returns the
    /// number of escrows actually swept.
    pub fn expire_trial_offers(env: Env, limit: u32) -> Result<u32, ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        let cap = limit.min(EXPIRE_TRIAL_OFFERS_MAX_LIMIT);
        let index_key = DataKey::OutstandingTrialEscrows;
        let outstanding: soroban_sdk::Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&index_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));

        let now = env.ledger().timestamp();
        let token_addr = Self::get_token(&env)?;
        let contract_addr = env.current_contract_address();

        let process_count = cap.min(outstanding.len());
        let mut kept: soroban_sdk::Vec<(u64, u32)> = soroban_sdk::Vec::new(&env);
        let mut swept = 0u32;

        // Only the first `process_count` entries are examined per call —
        // this bounds the work done regardless of total backlog size.
        for i in 0..process_count {
            let (player_id, index) = outstanding.get(i).unwrap();
            let escrow_key = DataKey::TrialEscrow(player_id, index);
            let escrow: Option<TrialEscrow> = env.storage().persistent().get(&escrow_key);
            let escrow = match escrow {
                Some(e) => e,
                // Already cleaned up by confirm_trial_offer; drop the stale
                // index entry instead of re-queueing it.
                None => continue,
            };
            if now <= escrow.expires_at {
                // Keep-alive: this escrow has not yet expired so it stays in
                // the outstanding list for a future sweep. Re-extend its TTL
                // so that a large backlog of live escrows cannot be silently
                // archived between sweep calls — which would make them
                // unreadable to both this sweep function and
                // confirm_trial_offer before the player ever acts on the offer.
                env.storage().persistent().extend_ttl(
                    &escrow_key,
                    TRIAL_TTL_THRESHOLD,
                    TRIAL_TTL_EXTEND_TO,
                );
                kept.push_back((player_id, index));
                continue;
            }

            let offer: Option<TrialOffer> = env
                .storage()
                .persistent()
                .get(&DataKey::TrialOffer(player_id, index));
            let scout = match offer {
                Some(o) => o.scout,
                // No originating offer to refund against; drop the orphaned
                // escrow entry without a transfer.
                None => {
                    env.storage().persistent().remove(&escrow_key);
                    continue;
                }
            };

            // ISSUE #1138: Check balance before transferring to prevent running
            // out of funds for other escrow refunds
            let balance = token::Client::new(&env, &token_addr).balance(&contract_addr);
            if escrow.amount > balance {
                // Keep this escrow for a later sweep when balance is replenished
                kept.push_back((player_id, index));
                continue;
            }

            token::Client::new(&env, &token_addr).transfer(&contract_addr, &scout, &escrow.amount);
            // ISSUE #1138: Decrement escrowed total on sweep
            let escrow_total_key = DataKey::EscrowedTotal;
            let current_escrowed: i128 = env.storage().instance().get(&escrow_total_key).unwrap_or(0i128);
            let new_escrowed = current_escrowed.saturating_sub(escrow.amount);
            env.storage().instance().set(&escrow_total_key, &new_escrowed);
            env.storage().persistent().remove(&escrow_key);
            events::trial_offer_expired(&env, player_id, &scout, index);
            swept = safe_add_u32(swept, 1).map_err(|_| ScoutAccessError::Overflow)?;
        }

        // Entries beyond process_count were never examined this call and
        // must be preserved for the next sweep.
        for i in process_count..outstanding.len() {
            kept.push_back(outstanding.get(i).unwrap());
        }

        env.storage().persistent().set(&index_key, &kept);
        env.storage()
            .persistent()
            .extend_ttl(&index_key, TRIAL_TTL_THRESHOLD, TRIAL_TTL_EXTEND_TO);

        Ok(swept)
    }

    /// Admin-only rescue valve: directly refund one identified, still
    /// outstanding `TrialEscrow` entry — e.g. one flagged by a scout
    /// complaint or surfaced by the indexer's drift-detection — without
    /// waiting for `expire_trial_offers` to reach it. See
    /// docs/TRIAL_ESCROW_IMPACT.md, recommendation 2.
    ///
    /// Rejects with `TrialEscrowNotOutstanding` if `(player_id, offer_index)`
    /// has no live `TrialEscrow` entry: already confirmed, already
    /// expired/refunded, already admin-refunded, or never logged. Because the
    /// check and the removal target the same record, a retried call after a
    /// successful refund finds nothing outstanding and safely no-ops into
    /// that same error instead of transferring a second time.
    pub fn admin_refund_trial_escrow(
        env: Env,
        player_id: u64,
        offer_index: u32,
        to: Address,
    ) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        let escrow_key = DataKey::TrialEscrow(player_id, offer_index);
        let escrow: TrialEscrow = env
            .storage()
            .persistent()
            .get(&escrow_key)
            .ok_or(ScoutAccessError::TrialEscrowNotOutstanding)?;

        let token_addr = Self::get_token(&env)?;
        let contract_addr = env.current_contract_address();
        let balance = token::Client::new(&env, &token_addr).balance(&contract_addr);
        if escrow.amount > balance {
            return Err(ScoutAccessError::InsufficientFee);
        }
        token::Client::new(&env, &token_addr).transfer(&contract_addr, &to, &escrow.amount);

        // ISSUE #1138: Decrement escrowed total on admin refund
        let escrow_total_key = DataKey::EscrowedTotal;
        let current_escrowed: i128 = env.storage().instance().get(&escrow_total_key).unwrap_or(0i128);
        let new_escrowed = current_escrowed.saturating_sub(escrow.amount);
        env.storage().instance().set(&escrow_total_key, &new_escrowed);

        // Same cleanup order as confirm_trial_offer/expire_trial_offers:
        // drop the primary record, then scrub the enumeration index, so no
        // later sweep or late confirm can see or act on this entry again.
        env.storage().persistent().remove(&escrow_key);
        Self::remove_from_outstanding_trial_escrows(&env, player_id, offer_index);

        events::trial_escrow_admin_refunded(&env, player_id, offer_index, &to, escrow.amount);
        Ok(())
    }

    /// Propose a replacement administrator. The current admin remains active
    /// until the proposed address calls `accept_admin`.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let old_admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage()
            .persistent()
            .set(&DataKey::PendingAdmin, &new_admin);
        env.storage().persistent().extend_ttl(
            &DataKey::PendingAdmin,
            ADMIN_BUMP_LEDGERS,
            ADMIN_BUMP_LEDGERS,
        );
        events::admin_transfer_proposed(&env, &old_admin, &new_admin);
        Ok(())
    }

    /// Accept a pending admin transfer. Only the proposed address can accept.
    pub fn accept_admin(env: Env) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ScoutAccessError::NotInitialized)?;
        let new_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdmin)
            .ok_or(ScoutAccessError::PendingAdminNotSet)?;
        new_admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &new_admin);
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ADMIN_BUMP_LEDGERS,
            ADMIN_BUMP_LEDGERS,
        );
        env.storage().persistent().remove(&DataKey::PendingAdmin);
        events::admin_transferred(&env, &old_admin, &new_admin);
        Ok(())
    }

    /// Deprecated alias for `propose_admin`; this no longer transfers control
    /// immediately. The proposed address must still call `accept_admin`.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), ScoutAccessError> {
        Self::propose_admin(env, new_admin)
    }

    // -------------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------------

    pub fn get_subscription(env: Env, scout: Address) -> Result<Subscription, ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let sub = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(scout.clone()))
            .ok_or(ScoutAccessError::ScoutNotSubscribed)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Subscription(scout),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        Ok(sub)
    }

    /// Recover an archived (or expired-but-not-evicted) subscription entry by
    /// re-extending its TTL to the core-identity policy value (518,400 ledgers).
    ///
    /// On Soroban protocol 23+, reading an archived entry auto-restores it
    /// within the archival grace period. This entrypoint makes that recovery
    /// explicit and operator-driven, then lifts the entry's TTL back to the
    /// full documented lifetime so it cannot silently age into permanent
    /// eviction.
    ///
    /// Admin-only. Returns `SubscriptionRecordEvicted` if the entry has already
    /// been fully evicted (key absent) and is unrecoverable.
    pub fn restore_subscription_record(env: Env, scout: Address) -> Result<(), ScoutAccessError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let _sub: Subscription = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(scout.clone()))
            .ok_or(ScoutAccessError::SubscriptionRecordEvicted)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Subscription(scout.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        events::subscription_record_restored(&env, &admin, &scout);
        Ok(())
    }

    pub fn get_fee_config(env: Env) -> FeeConfig {
        Self::bump_instance_ttl(&env);
        Self::fee_config(&env)
    }

    /// Return the bounded on-chain history of the last (up to 5) `FeeConfig`
    /// values **oldest-first**.
    ///
    /// Each entry contains the config that was active *before* a particular
    /// `update_fee_config` call, together with the timestamp at which that
    /// change was made. The *current* config is not included — retrieve it
    /// with `get_fee_config`.
    ///
    /// This is a lightweight middle-ground between the indexer-only design
    /// (full history replayed from `fee_config_updated` events) and a fuller
    /// unbounded ring-buffer: it makes the immediately-previous configs
    /// cheaply readable on-chain without depending on the off-chain indexer.
    /// The cap is fixed at 5 entries. When the cap is reached the oldest
    /// entry is evicted on the next `update_fee_config` call.
    pub fn get_fee_config_history(env: Env) -> Vec<FeeConfigHistoryEntry> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&DataKey::FeeConfigHistory)
            .unwrap_or_else(|| Vec::new(&env))
    }

    pub fn get_accumulated_fees(env: Env) -> i128 {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&DataKey::AccumulatedFees)
            .unwrap_or(0i128)
    }

    pub fn get_subscribers_by_tier(env: Env, tier: SubscriptionTier) -> soroban_sdk::Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::TierSubscribers(tier))
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
    }

    /// Return subscriptions whose `expires_at` is at or before `before_timestamp`,
    /// excluding any subscription that has already been renewed (i.e. whose stored
    /// `expires_at` is later than `before_timestamp`).
    ///
    /// Uses the day-granularity `ExpiryBucket` index populated by `subscribe` to
    /// avoid a full linear scan of all subscribers.  Only buckets whose day key
    /// falls within `[start_day, before_timestamp / 86_400]` are examined, where
    /// `start_day` is the minimum populated bucket day (`DataKey::MinExpiryBucketDay`)
    /// tracked by `add_to_expiry_bucket`.  Because no bucket is ever populated
    /// before that day, the scan is bounded by the number of distinct populated
    /// expiry days in range, not the number of days since epoch — in particular it
    /// does not waste instructions stepping through the (usually empty) day buckets
    /// between epoch 0 and the first real subscription.
    ///
    /// **Index tradeoff**: day-bucket granularity is chosen over exact expiry-time
    /// indexing to keep per-`subscribe` storage cost low.  Each bucket entry is
    /// one `Address` in a `Vec<Address>` stored under a single persistent key per
    /// day.  A scout is moved to a new bucket on every renewal, so stale entries
    /// do not accumulate indefinitely.  Callers receive only subscriptions whose
    /// real `expires_at` satisfies the predicate — the bucket is just a
    /// pre-filter, not the authoritative answer.
    ///
    /// `limit` is capped at `MAX_EXPIRY_PAGE_SIZE` (50) to bound CPU cost per call.
    /// Page through results by advancing `before_timestamp` by one second past
    /// the latest `expires_at` in the previous page.
    pub fn get_expiring_subscriptions(
        env: Env,
        before_timestamp: u64,
        limit: u32,
    ) -> soroban_sdk::Vec<Subscription> {
        Self::bump_instance_ttl(&env);

        const MAX_EXPIRY_PAGE_SIZE: u32 = 50;
        const SECS_PER_DAY: u64 = 86_400;

        let effective_limit = limit.min(MAX_EXPIRY_PAGE_SIZE);
        let cutoff_day = before_timestamp / SECS_PER_DAY;

        let mut results: soroban_sdk::Vec<Subscription> = soroban_sdk::Vec::new(&env);

        // Walk day buckets from the earliest populated bucket day up to
        // cutoff_day (inclusive). Skipping the (usually empty) day buckets
        // before MinExpiryBucketDay keeps this O(populated days), not
        // O(elapsed days since epoch).
        let start_day: u64 = env
            .storage()
            .instance()
            .get::<DataKey, u64>(&DataKey::MinExpiryBucketDay)
            .unwrap_or(0);
        let mut day = start_day;
        while day <= cutoff_day && results.len() < effective_limit {
            let bucket_key = DataKey::ExpiryBucket(day);
            if let Some(scouts) = env
                .storage()
                .persistent()
                .get::<DataKey, soroban_sdk::Vec<Address>>(&bucket_key)
            {
                let n = scouts.len();
                let mut i = 0u32;
                while i < n && results.len() < effective_limit {
                    let scout = scouts.get(i).unwrap();
                    // Re-read the live Subscription to get the current expires_at.
                    // This handles renewals: if the scout renewed, their bucket
                    // entry was moved and the current expires_at will be later
                    // than before_timestamp, so they are filtered out here.
                    if let Some(sub) = env
                        .storage()
                        .persistent()
                        .get::<DataKey, Subscription>(&DataKey::Subscription(scout.clone()))
                    {
                        if sub.expires_at <= before_timestamp {
                            results.push_back(sub);
                        }
                    }
                    i = i.saturating_add(1);
                }
            }
            day = day.saturating_add(1);
        }

        results
    }

    pub fn has_contacted(env: Env, scout: Address, player_id: u64) -> bool {
        Self::bump_instance_ttl(&env);
        let key = DataKey::ContactRecord(player_id, scout);
        let record: Option<ContactRecord> = env.storage().persistent().get(&key);
        if record.is_some() {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        record.is_some()
    }

    /// Retrieve the full ContactRecord for a (player_id, scout) pair.
    /// Returns None if the scout has not contacted this player.
    pub fn get_contact_record(env: Env, scout: Address, player_id: u64) -> Option<ContactRecord> {
        Self::bump_instance_ttl(&env);
        let key = DataKey::ContactRecord(player_id, scout);
        let record: Option<ContactRecord> = env.storage().persistent().get(&key);
        if record.is_some() {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        record
    }

    /// Return all player_ids contacted by `scout` as an O(1) index lookup.
    ///
    /// > **Deprecated**: this legacy method is unbounded.  High-volume callers
    /// should use [`get_scout_contacts_page`] to keep response sizes bounded.
    pub fn get_scout_contacts(env: Env, scout: Address) -> soroban_sdk::Vec<u64> {
        Self::bump_instance_ttl(&env);
        let key = DataKey::ScoutContacts(scout.clone());
        let list = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if !list.is_empty() {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        list
    }

    /// Return a bounded, paginated page of player IDs contacted by `scout`,
    /// together with the total number of contacts.
    ///
    /// This is the canonical paginated successor to the unbounded
    /// `get_scout_contacts`.  The `total` field lets callers determine when
    /// paging is complete without over-fetching.
    ///
    /// **Pagination**: `offset` is a zero-based item offset; `limit` is capped
    /// at 50 entries per page, matching the convention used by
    /// `get_global_milestone_index` and `get_validator_milestones_page_v2`.
    ///
    /// **Ordering**: entries are returned in contact order (oldest first).
    pub fn get_scout_contacts_page(
        env: Env,
        scout: Address,
        offset: u32,
        limit: u32,
    ) -> ScoutContactsPage {
        Self::bump_instance_ttl(&env);
        let key = DataKey::ScoutContacts(scout.clone());
        let list: soroban_sdk::Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if !list.is_empty() {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        let total = list.len();
        let cap = limit.min(50);
        let mut entries: soroban_sdk::Vec<u64> = soroban_sdk::Vec::new(&env);
        let mut i = offset;
        while i < total && entries.len() < cap {
            entries.push_back(list.get(i).unwrap());
            i += 1;
        }
        ScoutContactsPage { entries, total }
    }

    /// Return all scout addresses that have contacted `player_id` as an O(1)
    /// index lookup.  Players can audit their inbound contact history directly
    /// from on-chain state without replaying off-chain events.
    pub fn get_player_contacts(env: Env, player_id: u64) -> soroban_sdk::Vec<Address> {
        Self::bump_instance_ttl(&env);
        let key = DataKey::PlayerContacts(player_id);
        let list = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if !list.is_empty() {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        list
    }

    // -------------------------------------------------------------------------
    // Evidence access grants (docs/EVIDENCE_PRIVACY.md)
    // -------------------------------------------------------------------------

    /// True if `scout` currently holds a non-revoked `EvidenceAccessGrant`
    /// for `player_id`. The off-chain key-wrapping service calls this (or
    /// `get_evidence_access_grant`) before honoring a key-wrap request.
    pub fn has_evidence_access(env: Env, player_id: u64, scout: Address) -> bool {
        env.storage()
            .persistent()
            .get::<DataKey, EvidenceAccessGrant>(&DataKey::EvidenceAccessGrant(player_id, scout))
            .map(|g| !g.revoked)
            .unwrap_or(false)
    }

    /// Return the full `EvidenceAccessGrant` record for (player_id, scout),
    /// if one has ever been issued — including a revoked grant, so callers
    /// can distinguish "never granted" from "granted, then revoked".
    pub fn get_evidence_access_grant(
        env: Env,
        player_id: u64,
        scout: Address,
    ) -> Option<EvidenceAccessGrant> {
        env.storage()
            .persistent()
            .get(&DataKey::EvidenceAccessGrant(player_id, scout))
    }

    /// Page through every `EvidenceAccessGrant` ever issued for `player_id`,
    /// oldest-first, so a player-facing UI can audit who has access to their
    /// evidence. `limit` is capped at `MAX_ACCESS_GRANT_PAGE_LIMIT` (50) and
    /// equals `ACCESS_GRANT_PAGE_SIZE`, so a single call reads at most two
    /// index pages (the tail of one, the head of the next) plus one grant
    /// record per returned entry — CPU cost bounded by `limit`, independent
    /// of how many grants `player_id` has accumulated in total (proven at
    /// 1,000+ grants by `contracts/scout_access/tests/cost_budget.rs`).
    ///
    /// Page through a player's full history by advancing `offset` by the
    /// number of entries returned in the previous page.
    pub fn get_player_access_grants(
        env: Env,
        player_id: u64,
        offset: u32,
        limit: u32,
    ) -> soroban_sdk::Vec<EvidenceAccessGrant> {
        Self::bump_instance_ttl(&env);
        let mut results: soroban_sdk::Vec<EvidenceAccessGrant> = soroban_sdk::Vec::new(&env);

        let effective_limit = limit.min(MAX_ACCESS_GRANT_PAGE_LIMIT);
        if effective_limit == 0 {
            return results;
        }
        let end = match offset.checked_add(effective_limit) {
            Some(e) => e,
            None => return results,
        };

        let mut position = offset;
        while position < end {
            let page_idx = position / ACCESS_GRANT_PAGE_SIZE;
            let page: soroban_sdk::Vec<Address> = match env
                .storage()
                .persistent()
                .get(&DataKey::EvidenceAccessGrantPage(player_id, page_idx))
            {
                Some(p) => p,
                // Pages are appended contiguously with no gaps, so a missing
                // page means the enumeration ends here.
                None => break,
            };
            let mut i = position % ACCESS_GRANT_PAGE_SIZE;
            if i >= page.len() {
                break;
            }
            while i < page.len() && position < end {
                let scout = page.get(i).unwrap();
                if let Some(grant) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, EvidenceAccessGrant>(&DataKey::EvidenceAccessGrant(
                        player_id,
                        scout,
                    ))
                {
                    results.push_back(grant);
                }
                i = i.saturating_add(1);
                position = position.saturating_add(1);
            }
        }

        results
    }

    /// Compliance/abuse takedown: mark an `EvidenceAccessGrant` revoked.
    ///
    /// This does not delete the grant record — it is an append-only fact
    /// that this scout *was* authorized at `granted_at`. Revoking only
    /// instructs the off-chain key-wrapping service to stop honoring
    /// *future* key-wrap requests for this (player_id, scout) pair; it
    /// cannot claw back a wrapped key already delivered before the revoke
    /// (the contract never held the key — see `docs/EVIDENCE_PRIVACY.md`).
    /// Idempotent: revoking an already-revoked grant is a no-op that
    /// returns `Ok(())` without re-emitting the event.
    pub fn admin_revoke_evidence_access(
        env: Env,
        player_id: u64,
        scout: Address,
    ) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        let grant_key = DataKey::EvidenceAccessGrant(player_id, scout.clone());
        let mut grant: EvidenceAccessGrant = env
            .storage()
            .persistent()
            .get(&grant_key)
            .ok_or(ScoutAccessError::GrantNotFound)?;

        if grant.revoked {
            return Ok(());
        }

        grant.revoked = true;
        grant.revoked_at = Some(env.ledger().timestamp());
        env.storage().persistent().set(&grant_key, &grant);
        env.storage()
            .persistent()
            .extend_ttl(&grant_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        events::evidence_access_revoked(&env, player_id, &scout, &admin);
        Ok(())
    }

    pub fn get_trial_offer(
        env: Env,
        player_id: u64,
        index: u32,
    ) -> Result<TrialOffer, ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        let offer = env
            .storage()
            .persistent()
            .get(&DataKey::TrialOffer(player_id, index))
            .ok_or(ScoutAccessError::TrialOfferNotFound)?;
        env.storage().persistent().extend_ttl(
            &DataKey::TrialOffer(player_id, index),
            TRIAL_TTL_THRESHOLD,
            TRIAL_TTL_EXTEND_TO,
        );
        Ok(offer)
    }

    /// Return the escrow record for a pending trial offer, if it exists.
    /// This read-only getter is used by address-migration tooling to preserve
    /// in-flight escrow state instead of replaying only the offer metadata.
    pub fn get_trial_escrow(env: Env, player_id: u64, index: u32) -> Option<TrialEscrow> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .persistent()
            .get(&DataKey::TrialEscrow(player_id, index))
    }

    pub fn get_trial_count(env: Env, player_id: u64) -> u32 {
        Self::bump_instance_ttl(&env);
        let count = env
            .storage()
            .persistent()
            .get(&DataKey::TrialCounter(player_id))
            .unwrap_or(0u32);
        if count > 0 {
            env.storage().persistent().extend_ttl(
                &DataKey::TrialCounter(player_id),
                TRIAL_TTL_THRESHOLD,
                TRIAL_TTL_EXTEND_TO,
            );
        }
        count
    }

    /// Return all trial offers for a given player in ascending index order (1..=N).
    /// Returns an empty Vec for a player with no trial offers.
    pub fn get_player_trial_offers(env: Env, player_id: u64) -> soroban_sdk::Vec<TrialOffer> {
        Self::bump_instance_ttl(&env);
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::TrialCounter(player_id))
            .unwrap_or(0u32);
        let mut offers: soroban_sdk::Vec<TrialOffer> = soroban_sdk::Vec::new(&env);
        for i in 1..=count {
            if let Some(offer) = env
                .storage()
                .persistent()
                .get(&DataKey::TrialOffer(player_id, i))
            {
                offers.push_back(offer);
            }
        }
        offers
    }

    /// Return all trial offers for a player in a single call.
    /// Bounded at 20 to prevent gas exhaustion. Returns empty Vec for no offers.
    pub fn get_all_trial_offers(env: Env, player_id: u64) -> soroban_sdk::Vec<TrialOffer> {
        const MAX_OFFERS: u32 = 20;
        Self::bump_instance_ttl(&env);

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::TrialCounter(player_id))
            .unwrap_or(0u32);

        let limit = count.min(MAX_OFFERS);
        let mut offers: soroban_sdk::Vec<TrialOffer> = soroban_sdk::Vec::new(&env);
        for i in 1..=limit {
            if let Some(offer) = env
                .storage()
                .persistent()
                .get(&DataKey::TrialOffer(player_id, i))
            {
                offers.push_back(offer);
            }
        }
        offers
    }

    // -------------------------------------------------------------------------
    // Migration window management
    // -------------------------------------------------------------------------

    /// Open the migration window.  Admin-only.
    ///
    /// While open, `admin_seed_subscription`, `admin_seed_contact`,
    /// `admin_seed_trial_offer`, and `admin_seed_auto_renew` may be called to
    /// replay historical records.  Close immediately after replay.
    pub fn open_migration_window(env: Env) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MigrationActive, &true);
        Ok(())
    }

    /// Close the migration window.  Admin-only.
    pub fn close_migration_window(env: Env) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MigrationActive, &false);
        Ok(())
    }

    /// Returns `true` if the migration window is currently open.
    pub fn migration_window_is_open(env: Env) -> bool {
        env.storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::MigrationActive)
            .unwrap_or(false)
    }

    // -------------------------------------------------------------------------
    // Migration seeding
    // -------------------------------------------------------------------------

    /// Seed a historical scout `Subscription` from a prior contract deployment.
    ///
    /// MIGRATION_GAPS row 8.  Reconstructs:
    ///
    /// - `Subscription(scout)` — the subscription record
    /// - `TierSubscribers(tier)` — tier-indexed subscriber list
    /// - `ExpiryBucket(bucket)` — day-granularity expiry index
    ///
    /// ## Idempotency
    ///
    /// Keyed on `scout` address.  Byte-identical replay → no-op.
    /// Conflicting content → `SubscriptionAlreadyExists`.
    pub fn admin_seed_subscription(
        env: Env,
        subscription: Subscription,
    ) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        let sub_key = DataKey::Subscription(subscription.scout.clone());

        // ── Idempotency ───────────────────────────────────────────────────────
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, Subscription>(&sub_key)
        {
            let identical = existing.scout == subscription.scout
                && existing.tier == subscription.tier
                && existing.expires_at == subscription.expires_at
                && existing.subscribed_at == subscription.subscribed_at;
            if identical {
                return Ok(());
            }
            return Err(ScoutAccessError::SubscriptionAlreadyExists);
        }

        // ── Write Subscription ────────────────────────────────────────────────
        env.storage().persistent().set(&sub_key, &subscription);
        env.storage()
            .persistent()
            .extend_ttl(&sub_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Update TierSubscribers ────────────────────────────────────────────
        let ts_key = DataKey::TierSubscribers(subscription.tier.clone());
        let mut tier_subs: Vec<Address> = env
            .storage()
            .persistent()
            .get(&ts_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !tier_subs.iter().any(|a| a == subscription.scout) {
            tier_subs.push_back(subscription.scout.clone());
            env.storage().persistent().set(&ts_key, &tier_subs);
            env.storage()
                .persistent()
                .extend_ttl(&ts_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        // ── Update ExpiryBucket ───────────────────────────────────────────────
        let bucket = subscription.expires_at / 86_400;
        let eb_key = DataKey::ExpiryBucket(bucket);
        let mut bucket_scouts: Vec<Address> = env
            .storage()
            .persistent()
            .get(&eb_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !bucket_scouts.iter().any(|a| a == subscription.scout) {
            bucket_scouts.push_back(subscription.scout.clone());
            env.storage().persistent().set(&eb_key, &bucket_scouts);
            env.storage()
                .persistent()
                .extend_ttl(&eb_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        Self::track_min_expiry_bucket_day(&env, bucket);

        Ok(())
    }

    /// Seed a historical contact record from a prior contract deployment.
    ///
    /// MIGRATION_GAPS row 9.  Reconstructs:
    ///
    /// - `ContactRecord(player_id, scout)` — full contact record
    /// - `ScoutContacts(scout)` — per-scout contacted-player list
    /// - `PlayerContacts(player_id)` — per-player contacting-scout list
    ///
    /// ## Idempotency
    ///
    /// Keyed on `(player_id, scout)`.  Byte-identical replay → no-op;
    /// conflicting content → `ContactAlreadyExists`.
    pub fn admin_seed_contact(env: Env, contact: ContactRecord) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        let cr_key = DataKey::ContactRecord(contact.player_id, contact.scout.clone());

        // ── Idempotency ───────────────────────────────────────────────────────
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, ContactRecord>(&cr_key)
        {
            let identical = existing.player_id == contact.player_id
                && existing.scout == contact.scout
                && existing.contacted_at == contact.contacted_at;
            if identical {
                return Ok(());
            }
            return Err(ScoutAccessError::ContactAlreadyExists);
        }

        // ── Write ContactRecord ───────────────────────────────────────────────
        env.storage().persistent().set(&cr_key, &contact);
        env.storage()
            .persistent()
            .extend_ttl(&cr_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Update ScoutContacts ──────────────────────────────────────────────
        let sc_key = DataKey::ScoutContacts(contact.scout.clone());
        let mut sc: Vec<u64> = env
            .storage()
            .persistent()
            .get(&sc_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !sc.iter().any(|id| id == contact.player_id) {
            sc.push_back(contact.player_id);
            env.storage().persistent().set(&sc_key, &sc);
            env.storage()
                .persistent()
                .extend_ttl(&sc_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        // ── Update PlayerContacts ─────────────────────────────────────────────
        let pc_key = DataKey::PlayerContacts(contact.player_id);
        let mut pc: Vec<Address> = env
            .storage()
            .persistent()
            .get(&pc_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !pc.iter().any(|a| a == contact.scout) {
            pc.push_back(contact.scout.clone());
            env.storage().persistent().set(&pc_key, &pc);
            env.storage()
                .persistent()
                .extend_ttl(&pc_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        Ok(())
    }

    /// Seed a historical trial offer from a prior contract deployment.
    ///
    /// MIGRATION_GAPS row 10.  Reconstructs:
    ///
    /// - `TrialOffer(player_id, trial_index)` — the offer record
    /// - `TrialEscrow(player_id, trial_index)` — escrow record (if provided)
    /// - `TrialCounter(player_id)` — per-player offer count
    /// - `ScoutTrialOffers(scout)` — per-scout offers-sent index
    ///
    /// `trial_index` is 1-based, matching `log_trial_offer`: the first offer
    /// a scout logs is index 1 and each subsequent seed must be
    /// `TrialCounter(player_id) + 1`.  Zero, gaps, or out-of-order indices
    /// return `TrialOfferNotFound`.
    ///
    /// ## Idempotency
    ///
    /// Keyed on `(player_id, trial_index)`.  Byte-identical replay → no-op.
    /// Conflicting content → `TrialOfferAlreadyExists`.
    pub fn admin_seed_trial_offer(
        env: Env,
        player_id: u64,
        trial_index: u32,
        offer: TrialOffer,
        escrow: Option<TrialEscrow>,
    ) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        let to_key = DataKey::TrialOffer(player_id, trial_index);

        // ── Idempotency ───────────────────────────────────────────────────────
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, TrialOffer>(&to_key)
        {
            let identical = existing.player_id == offer.player_id
                && existing.scout == offer.scout
                && existing.details_hash == offer.details_hash
                && existing.logged_at == offer.logged_at;
            if identical {
                return Ok(());
            }
            return Err(ScoutAccessError::TrialOfferAlreadyExists);
        }

        // ── Index continuity (1-based, matching log_trial_offer) ─────────────
        let tc_key = DataKey::TrialCounter(player_id);
        let current_count: u32 = env.storage().persistent().get(&tc_key).unwrap_or(0u32);
        let expected_index =
            safe_add_u32(current_count, 1).map_err(|_| ScoutAccessError::Overflow)?;
        if trial_index != expected_index {
            return Err(ScoutAccessError::TrialOfferNotFound);
        }

        // ── Write TrialOffer ──────────────────────────────────────────────────
        env.storage().persistent().set(&to_key, &offer);
        env.storage()
            .persistent()
            .extend_ttl(&to_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Write TrialEscrow (if provided) ───────────────────────────────────
        if let Some(escrow_rec) = escrow {
            let te_key = DataKey::TrialEscrow(player_id, trial_index);
            env.storage().persistent().set(&te_key, &escrow_rec);
            env.storage()
                .persistent()
                .extend_ttl(&te_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        // ── Increment TrialCounter ────────────────────────────────────────────
        env.storage().persistent().set(&tc_key, &trial_index);
        env.storage()
            .persistent()
            .extend_ttl(&tc_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Update ScoutTrialOffers ───────────────────────────────────────────
        let sto_key = DataKey::ScoutTrialOffers(offer.scout.clone());
        let mut sto: Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&sto_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !sto
            .iter()
            .any(|(pid, tidx)| pid == player_id && tidx == trial_index)
        {
            sto.push_back((player_id, trial_index));
            env.storage().persistent().set(&sto_key, &sto);
            env.storage()
                .persistent()
                .extend_ttl(&sto_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        Ok(())
    }

    /// Seed the auto-renewal opt-in flag for a scout.
    ///
    /// MIGRATION_GAPS row 13.  Reconstructs `AutoRenew(scout)` → bool.
    ///
    /// ## Idempotency
    ///
    /// If the same `enabled` value is already stored → no-op.
    /// If a *different* value is stored → `AutoRenewAlreadyExists`.
    pub fn admin_seed_auto_renew(
        env: Env,
        scout: Address,
        enabled: bool,
    ) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        let ar_key = DataKey::AutoRenew(scout.clone());

        // ── Idempotency ───────────────────────────────────────────────────────
        if let Some(existing) = env.storage().persistent().get::<DataKey, bool>(&ar_key) {
            if existing == enabled {
                return Ok(());
            }
            return Err(ScoutAccessError::AutoRenewAlreadyExists);
        }

        env.storage().persistent().set(&ar_key, &enabled);
        env.storage()
            .persistent()
            .extend_ttl(&ar_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        Ok(())
    }

    /// Seed the current fee configuration and its bounded history from a prior
    /// deployment. This is intentionally one atomic replacement rather than a
    /// loop of `update_fee_config` calls: replay preserves the original
    /// timestamps and does not manufacture fee-config events or delays.
    pub fn admin_seed_fee_config(
        env: Env,
        config: FeeConfig,
        history: Vec<FeeConfigHistoryEntry>,
    ) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        const HISTORY_CAP: u32 = 5;
        if history.len() > HISTORY_CAP {
            return Err(ScoutAccessError::InvalidInput);
        }

        let history_key = DataKey::FeeConfigHistory;
        if let Some(existing) = env
            .storage()
            .instance()
            .get::<DataKey, Vec<FeeConfigHistoryEntry>>(&history_key)
        {
            let identical = existing.len() == history.len()
                && (0..existing.len()).all(|i| existing.get(i) == history.get(i));
            if !identical {
                return Err(ScoutAccessError::FeeConfigHistoryAlreadyExists);
            }
        }

        Self::validate_fee_config(&config)?;
        env.storage().instance().set(&DataKey::FeeConfig, &config);
        env.storage().instance().set(&history_key, &history);
        Ok(())
    }

    /// Seed only the bounded fee-configuration history when the current config
    /// was already restored by the deployment initializer.
    pub fn admin_seed_fee_config_history(
        env: Env,
        history: Vec<FeeConfigHistoryEntry>,
    ) -> Result<(), ScoutAccessError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        const HISTORY_CAP: u32 = 5;
        if history.len() > HISTORY_CAP {
            return Err(ScoutAccessError::InvalidInput);
        }

        let key = DataKey::FeeConfigHistory;
        if let Some(existing) = env
            .storage()
            .instance()
            .get::<DataKey, Vec<FeeConfigHistoryEntry>>(&key)
        {
            let identical = existing.len() == history.len()
                && (0..existing.len()).all(|i| existing.get(i) == history.get(i));
            if identical {
                return Ok(());
            }
            return Err(ScoutAccessError::FeeConfigHistoryAlreadyExists);
        }

        env.storage().instance().set(&key, &history);
        Ok(())
    }

    pub fn health(env: Env) -> ContractHealth {
        let initialized = env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Initialized)
            .unwrap_or(false);
        let paused = env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Paused)
            .unwrap_or(false);
        let pay_to_contact_paused = env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::PausedPayToContact)
            .unwrap_or(false);
        ContractHealth {
            initialized,
            paused,
            pay_to_contact_paused,
        }
    }

    /// Returns all (player_id, trial_index) tuples for every trial offer logged
    /// by `scout`. The returned Vec is in insertion order (oldest first).
    ///
    /// Returns an empty Vec for a scout who has not logged any trial offers.
    /// Each tuple can be passed to `get_trial_offer(player_id, index)` to fetch
    /// the full offer record (closes #468).
    pub fn get_scout_trial_offers(env: Env, scout: Address) -> soroban_sdk::Vec<(u64, u32)> {
        Self::bump_instance_ttl(&env);
        let key = DataKey::ScoutTrialOffers(scout.clone());
        let list = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if !list.is_empty() {
            env.storage()
                .persistent()
                .extend_ttl(&key, TRIAL_TTL_THRESHOLD, TRIAL_TTL_EXTEND_TO);
        }
        list
    }

    /// Returns the deployed crate version (from Cargo.toml at build time).
    pub fn version(env: Env) -> String {
        String::from_str(&env, CONTRACT_VERSION)
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    fn add_to_tier_index(env: &Env, scout: &Address, tier: &SubscriptionTier) {
        let key = DataKey::TierSubscribers(tier.clone());
        let mut subscribers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        if !subscribers.contains(scout) {
            subscribers.push_back(scout.clone());
        }
        env.storage().persistent().set(&key, &subscribers);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
    }

    fn remove_from_tier_index(env: &Env, scout: &Address, tier: &SubscriptionTier) {
        let key = DataKey::TierSubscribers(tier.clone());
        if let Some(subscribers) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<Address>>(&key)
        {
            let mut new_list: Vec<Address> = Vec::new(env);
            for i in 0..subscribers.len() {
                let addr = subscribers.get(i).unwrap();
                if &addr != scout {
                    new_list.push_back(addr);
                }
            }
            env.storage().persistent().set(&key, &new_list);
        }
    }

    /// Add `scout` to the day-granularity expiry bucket for `expires_at`.
    /// The bucket key is `expires_at / 86_400` so all subscriptions expiring
    /// on the same UTC day share a single persistent storage entry.
    fn add_to_expiry_bucket(env: &Env, scout: &Address, expires_at: u64) {
        const SECS_PER_DAY: u64 = 86_400;
        let day = expires_at / SECS_PER_DAY;
        let key = DataKey::ExpiryBucket(day);
        let mut bucket: Vec<Address> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        if !bucket.contains(scout) {
            bucket.push_back(scout.clone());
        }
        env.storage().persistent().set(&key, &bucket);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        Self::track_min_expiry_bucket_day(env, day);
    }

    /// Record the earliest populated expiry-bucket day so
    /// `get_expiring_subscriptions` can start its bucket scan there instead of
    /// at day 0. Only ever lowers the stored value (monotonic minimum), so it
    /// remains a safe lower bound even after a bucket later empties via
    /// `remove_from_expiry_bucket`.
    fn track_min_expiry_bucket_day(env: &Env, day: u64) {
        let current: u64 = env
            .storage()
            .instance()
            .get::<DataKey, u64>(&DataKey::MinExpiryBucketDay)
            .unwrap_or(u64::MAX);
        if day < current {
            env.storage()
                .instance()
                .set(&DataKey::MinExpiryBucketDay, &day);
        }
    }

    /// Remove `scout` from the day-granularity expiry bucket for `expires_at`.
    /// Called on subscription renewal so the old bucket entry does not produce
    /// false positives in `get_subscriptions_expiring_before` queries.
    fn remove_from_expiry_bucket(env: &Env, scout: &Address, expires_at: u64) {
        const SECS_PER_DAY: u64 = 86_400;
        let day = expires_at / SECS_PER_DAY;
        let key = DataKey::ExpiryBucket(day);
        if let Some(bucket) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<Address>>(&key)
        {
            let mut new_bucket: Vec<Address> = Vec::new(env);
            for i in 0..bucket.len() {
                let addr = bucket.get(i).unwrap();
                if &addr != scout {
                    new_bucket.push_back(addr);
                }
            }
            env.storage().persistent().set(&key, &new_bucket);
        }
    }

    fn require_initialized(env: &Env) -> Result<(), ScoutAccessError> {
        if !env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            return Err(ScoutAccessError::NotInitialized);
        }
        Ok(())
    }

    fn require_migration_active(env: &Env) -> Result<(), ScoutAccessError> {
        let active = env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::MigrationActive)
            .unwrap_or(false);
        if !active {
            return Err(ScoutAccessError::MigrationNotActive);
        }
        Ok(())
    }

    fn require_not_paused(env: &Env) -> Result<(), ScoutAccessError> {
        if env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(ScoutAccessError::ContractPaused);
        }
        Ok(())
    }

    /// Check that `pay_to_contact` is not paused (function-scoped circuit
    /// breaker). Independent of the whole-contract `Paused` flag; defaults to
    /// `false` (not paused) when the flag has never been set. Mirrors
    /// `verification::require_approve_milestone_not_paused`. (issue #1056)
    fn require_pay_to_contact_not_paused(env: &Env) -> Result<(), ScoutAccessError> {
        if env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::PausedPayToContact)
            .unwrap_or(false)
        {
            return Err(ScoutAccessError::PayToContactPaused);
        }
        Ok(())
    }

    fn get_token(env: &Env) -> Result<Address, ScoutAccessError> {
        env.storage()
            .instance()
            .get(&DataKey::XlmToken)
            .ok_or(ScoutAccessError::NotInitialized)
    }

    /// #795: drop `(player_id, index)` from the OutstandingTrialEscrows
    /// sweep index. Called whenever a TrialEscrow is cleaned up outside of
    /// `expire_trial_offers` itself, i.e. from `confirm_trial_offer`.
    fn remove_from_outstanding_trial_escrows(env: &Env, player_id: u64, index: u32) {
        let key = DataKey::OutstandingTrialEscrows;
        let outstanding: soroban_sdk::Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(env));
        let mut kept: soroban_sdk::Vec<(u64, u32)> = soroban_sdk::Vec::new(env);
        for i in 0..outstanding.len() {
            let entry = outstanding.get(i).unwrap();
            if entry != (player_id, index) {
                kept.push_back(entry);
            }
        }
        env.storage().persistent().set(&key, &kept);
        env.storage()
            .persistent()
            .extend_ttl(&key, TRIAL_TTL_THRESHOLD, TRIAL_TTL_EXTEND_TO);
    }

    fn require_active_subscription(
        env: &Env,
        scout: &Address,
    ) -> Result<Subscription, ScoutAccessError> {
        let sub: Subscription = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(scout.clone()))
            .ok_or(ScoutAccessError::ScoutNotSubscribed)?;
        if env.ledger().timestamp() > sub.expires_at {
            return Err(ScoutAccessError::SubscriptionExpired);
        }
        Ok(sub)
    }

    fn fee_config(env: &Env) -> FeeConfig {
        env.storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .expect("fee config not set")
    }

    fn accumulate_fee(env: &Env, amount: i128) -> Result<(), ScoutAccessError> {
        let current: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedFees)
            .unwrap_or(0i128);
        let new_total = safe_add_i128(current, amount).map_err(|_| ScoutAccessError::Overflow)?;
        env.storage()
            .instance()
            .set(&DataKey::AccumulatedFees, &new_total);
        Ok(())
    }

    /// Transfer `amount` stroops from `payer` to this contract and add it to
    /// `AccumulatedFees`. Both steps are atomic within the transaction.
    fn collect_fee(env: &Env, payer: &Address, amount: i128) -> Result<(), ScoutAccessError> {
        let xlm = Self::get_token(env)?;
        let contract_addr = env.current_contract_address();
        token::Client::new(env, &xlm).transfer(payer, &contract_addr, &amount);
        Self::accumulate_fee(env, amount)
    }

    /// Validate that every fee field is positive and durations are non-zero.
    ///
    /// This is the single authoritative validation entry point for `FeeConfig`.
    /// Both `initialize` and `update_fee_config` call this method, and any
    /// future field added to `FeeConfig` must be validated here.
    /// Validate that every fee field meets the minimum floor and sub_duration_secs is non-zero.
    fn validate_fee_config(config: &FeeConfig) -> Result<(), ScoutAccessError> {
        if config.contact_fee_stroops < MIN_CONTACT_FEE_STROOPS
            || config.basic_sub_stroops < MIN_SUB_FEE_STROOPS
            || config.pro_sub_stroops < MIN_SUB_FEE_STROOPS
            || config.elite_sub_stroops < MIN_SUB_FEE_STROOPS
            || config.sub_duration_secs == 0
            || config.trial_offer_escrow_stroops <= 0
            || config.trial_offer_expiry_secs == 0
            || config.pro_contact_limit == 0
        {
            return Err(ScoutAccessError::InvalidInput);
        }
        Ok(())
    }

    /// Numeric rank for a subscription tier (higher = more privileged).
    fn tier_rank(tier: &SubscriptionTier) -> u32 {
        match tier {
            SubscriptionTier::Basic => 1,
            SubscriptionTier::Pro => 2,
            SubscriptionTier::Elite => 3,
        }
    }
}

// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
        token::{Client as TokenClient, StellarAssetClient},
        Env, IntoVal, String, Symbol,
    };

    fn create_token(env: &Env, admin: &Address) -> Address {
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        token_id.address()
    }

    fn mint_token(env: &Env, token: &Address, _admin: &Address, to: &Address, amount: i128) {
        StellarAssetClient::new(env, token).mint(to, &amount);
    }

    fn default_fees() -> FeeConfig {
        FeeConfig {
            contact_fee_stroops: 100_000,
            basic_sub_stroops: 1_000_000,
            pro_sub_stroops: 3_000_000,
            elite_sub_stroops: 7_000_000,
            sub_duration_secs: 30 * 24 * 60 * 60,
            pro_contact_limit: 10,
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        }
    }

    fn setup() -> (
        Env,
        Address,
        Address,
        Address,
        ScoutAccessContractClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);
        client.initialize(&admin, &xlm, &default_fees());
        (env, admin, xlm, contract_id, client)
    }

    #[test]
    fn test_initialize_event() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);

        client.initialize(&admin, &xlm, &default_fees());

        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, crate::events::CONTRACT_INITIALIZED),
                        admin.clone()
                    )
                        .into_val(&env),
                    admin.clone().into_val(&env)
                )
            ]
        );

        let res = client.try_initialize(&admin, &xlm, &default_fees());
        assert_eq!(res, Err(Ok(ScoutAccessError::AlreadyInitialized)));

        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![&env]
        );
    }

    #[test]
    fn test_initialize_and_health() {
        let (_, _, _, _, client) = setup();
        assert!(client.health().initialized);
    }

    #[test]
    fn test_initialize_accepts_real_token_contract() {
        // A registered SAC exposes decimals() and must pass the probe.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);

        let res = client.try_initialize(&admin, &xlm, &default_fees());
        assert!(res.is_ok(), "real token contract should be accepted");
    }

    #[test]
    fn test_initialize_rejects_plain_account_as_xlm_token() {
        // A generated Address is a plain account, not a contract. The
        // decimals() probe must fail and initialize must return InvalidInput
        // with no storage side effects.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let not_a_token = Address::generate(&env);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);

        let res = client.try_initialize(&admin, &not_a_token, &default_fees());
        assert_eq!(res, Err(Ok(ScoutAccessError::InvalidInput)));
        assert!(!client.health().initialized);
    }

    #[test]
    fn test_initialize_rejects_non_token_contract_as_xlm_token() {
        // A registered contract that does not expose decimals() must also be
        // rejected. We register a fresh contract (the scout_access contract
        // itself) which has no decimals() method.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let not_a_token = env.register(ScoutAccessContract, ());
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);

        let res = client.try_initialize(&admin, &not_a_token, &default_fees());
        assert_eq!(res, Err(Ok(ScoutAccessError::InvalidInput)));
        assert!(!client.health().initialized);
    }

    #[test]
    fn test_fee_config_updated_event_contains_old_and_new_config() {
        let (env, _admin, _xlm, contract_id, client) = setup();

        let old_config = default_fees();
        let new_fees = FeeConfig {
            contact_fee_stroops: 200_000,
            basic_sub_stroops: 2_000_000,
            pro_sub_stroops: 5_000_000,
            elite_sub_stroops: 10_000_000,
            sub_duration_secs: 60 * 24 * 60 * 60,
            pro_contact_limit: 20,
            trial_offer_escrow_stroops: 1_000_000,
            trial_offer_expiry_secs: 7_200,
        };

        client.update_fee_config(&new_fees);

        // Assert that the fee_config_updated event was emitted with old and new
        // config, immediately followed by fee_config_delay_bypassed (#1055):
        // update_fee_config is the atomic, no-delay path, so it must flag
        // itself as such in the event stream, distinguishing it from an
        // activate_fee_config call (which emits fee_config_updated alone).
        // Checked immediately — `events().all()` only reflects the most
        // recent invocation, and the reads below are themselves separate invocations.
        let events = env.events().all().filter_by_contract(&contract_id);
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "fee_config_updated"), _admin.clone(),).into_val(&env),
                    (old_config.clone(), new_fees.clone()).into_val(&env)
                ),
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, "fee_config_delay_bypassed"),
                        _admin.clone(),
                    )
                        .into_val(&env),
                    (old_config, new_fees.clone()).into_val(&env)
                )
            ]
        );

        // Storage must reflect the new config.
        let stored = client.get_fee_config();
        assert_eq!(stored.contact_fee_stroops, new_fees.contact_fee_stroops);
        assert_eq!(stored.pro_contact_limit, new_fees.pro_contact_limit);
    }

    #[test]
    fn test_version() {
        let (env, _, _, _, client) = setup();
        assert_eq!(
            client.version(),
            String::from_str(&env, env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn test_subscribe_basic() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);

        client.subscribe(&scout, &SubscriptionTier::Basic);

        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Basic);
        assert!(sub.expires_at > sub.subscribed_at);
        assert_eq!(client.get_accumulated_fees(), 1_000_000);
    }

    #[test]
    fn test_subscribe_pro_tier() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Pro);
        assert!(sub.expires_at > sub.subscribed_at);
        assert_eq!(client.get_accumulated_fees(), 3_000_000);
    }

    #[test]
    fn test_scout_subscribed_event_includes_fee_paid() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);

        client.subscribe(&scout, &SubscriptionTier::Basic);

        // Both legacy and new events must be emitted, subscription_created
        // first, then legacy scout_subscribed. Checked immediately — `events().all()`
        // only reflects the most recent invocation, and `get_subscription` below
        // is itself a separate invocation.
        let emitted = env.events().all().filter_by_contract(&contract_id);
        let sub = client.get_subscription(&scout);
        assert_eq!(
            emitted,
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "subscription_created"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Basic, sub.subscribed_at, sub.expires_at).into_val(&env)
                ),
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "scout_subscribed"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Basic, default_fees().basic_sub_stroops).into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_scout_subscribed_event_fee_pro_tier() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Checked immediately — `events().all()` only reflects the most recent
        // invocation, and `get_subscription` below is itself a separate invocation.
        let emitted = env.events().all().filter_by_contract(&contract_id);
        let sub = client.get_subscription(&scout);
        assert_eq!(
            emitted,
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "subscription_created"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Pro, sub.subscribed_at, sub.expires_at).into_val(&env)
                ),
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "scout_subscribed"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Pro, default_fees().pro_sub_stroops).into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_subscribe_elite_and_pay_to_contact() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);

        assert!(client.has_contacted(&scout, &1u64));
        // elite fee + contact fee
        assert_eq!(client.get_accumulated_fees(), 7_000_000 + 100_000);
    }

    /// Issue #422 — contact_player stores a ContactRecord but no test reads back
    /// all fields and asserts correctness. This test calls pay_to_contact, then
    /// retrieves the stored ContactRecord via get_contact_record and asserts that
    /// player_id, scout address, and contacted_at all match the expected values.
    #[test]
    fn test_contact_record_fields_stored_correctly() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        let player_id: u64 = 77;

        // Pin the ledger timestamp so we can assert contacted_at precisely.
        env.ledger().with_mut(|l| l.timestamp = 1_500_000);

        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);

        // Advance time slightly so the contact timestamp is distinct from
        // the subscription timestamp, making the assertion unambiguous.
        env.ledger().with_mut(|l| l.timestamp = 1_500_100);
        let contact_time = env.ledger().timestamp();

        client.pay_to_contact(&scout, &player_id);

        // Retrieve and unwrap the stored record.
        let record = client
            .get_contact_record(&scout, &player_id)
            .expect("ContactRecord should exist after pay_to_contact");

        // All three fields must match exactly.
        assert_eq!(record.player_id, player_id, "player_id mismatch");
        assert_eq!(record.scout, scout, "scout address mismatch");
        assert!(
            record.contacted_at >= contact_time,
            "contacted_at ({}) should be >= ledger timestamp at call time ({})",
            record.contacted_at,
            contact_time,
        );

        // has_contacted must still return true (regression guard).
        assert!(client.has_contacted(&scout, &player_id));

        // get_contact_record for an unknown pair must return None.
        assert!(client.get_contact_record(&scout, &999u64).is_none());
    }

    #[test]
    fn test_player_contacted_event_includes_fee_paid() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &42u64);

        // pay_to_contact emits two events in this order:
        // 1. evidence_access_granted (EvidenceAccessGrant written atomically)
        // 2. player_contacted
        // Verify both events are present in the correct order.
        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, crate::events::EVIDENCE_ACCESS_GRANTED),
                        scout.clone()
                    )
                        .into_val(&env),
                    (42u64, SubscriptionTier::Elite).into_val(&env)
                ),
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, crate::events::PLAYER_CONTACTED),
                        scout.clone()
                    )
                        .into_val(&env),
                    (42u64, default_fees().contact_fee_stroops).into_val(&env)
                )
            ]
        );
    }

    #[test]
    #[should_panic]
    fn test_duplicate_contact_fails() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        // second contact with same player should panic
        client.pay_to_contact(&scout, &1u64);
    }

    #[test]
    fn test_player_contacts_index_updated_on_pay_to_contact() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout1 = Address::generate(&env);
        let scout2 = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout1, 100_000_000);
        mint_token(&env, &xlm, &admin, &scout2, 100_000_000);

        // Before any contact the inbound index is empty.
        assert_eq!(client.get_player_contacts(&1u64).len(), 0);

        // First scout contacts the player.
        client.subscribe(&scout1, &SubscriptionTier::Pro);
        client.pay_to_contact(&scout1, &1u64);

        let contacts = client.get_player_contacts(&1u64);
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts.get(0).unwrap(), scout1);

        // Second scout contacts the same player.
        client.subscribe(&scout2, &SubscriptionTier::Pro);
        client.pay_to_contact(&scout2, &1u64);

        let contacts = client.get_player_contacts(&1u64);
        assert_eq!(contacts.len(), 2);
        assert!(contacts.contains(&scout1));
        assert!(contacts.contains(&scout2));
    }

    #[test]
    fn test_player_contacts_not_duplicated_on_repeated_contact_attempt() {
        // The ContactRecord guard prevents a second pay_to_contact, so the
        // inbound index should never grow beyond the set of unique scouts.
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);
        client.pay_to_contact(&scout, &1u64);

        // Trying a second time should fail (AlreadyContacted), so the index stays at 1.
        let result = client.try_pay_to_contact(&scout, &1u64);
        assert!(result.is_err());

        assert_eq!(client.get_player_contacts(&1u64).len(), 1);
    }

    #[test]
    fn test_player_contacts_independent_per_player() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        // Scout contacts two different players.
        client.pay_to_contact(&scout, &1u64);
        client.pay_to_contact(&scout, &2u64);

        // Each player's inbound index contains only this scout.
        assert_eq!(client.get_player_contacts(&1u64).len(), 1);
        assert_eq!(client.get_player_contacts(&2u64).len(), 1);
        // Player 3 was never contacted.
        assert_eq!(client.get_player_contacts(&3u64).len(), 0);
    }

    #[test]
    fn test_pro_contact_limit_enforced() {
        // Set pro_contact_limit to 3 so we can hit it cheaply in a test.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);
        let fees = FeeConfig {
            contact_fee_stroops: 100_000,
            basic_sub_stroops: 1_000_000,
            pro_sub_stroops: 3_000_000,
            elite_sub_stroops: 7_000_000,
            sub_duration_secs: 30 * 24 * 60 * 60,
            pro_contact_limit: 3,
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        };
        client.initialize(&admin, &xlm, &fees);

        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Pro);

        // First 3 contacts succeed.
        client.pay_to_contact(&scout, &1u64);
        client.pay_to_contact(&scout, &2u64);
        client.pay_to_contact(&scout, &3u64);

        // Fourth contact must be rejected with ContactQuotaExceeded (#1161 unification).
        let res = client.try_pay_to_contact(&scout, &4u64);
        assert_eq!(res, Err(Ok(ScoutAccessError::ContactQuotaExceeded)));
    }

    #[test]
    fn test_pro_contact_limit_not_applied_to_elite() {
        // Elite scouts are unlimited — they must not hit the Pro quota.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);
        let fees = FeeConfig {
            contact_fee_stroops: 100_000,
            basic_sub_stroops: 1_000_000,
            pro_sub_stroops: 3_000_000,
            elite_sub_stroops: 7_000_000,
            sub_duration_secs: 30 * 24 * 60 * 60,
            pro_contact_limit: 2, // very low cap — Elite must ignore this
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        };
        client.initialize(&admin, &xlm, &fees);

        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);

        // Elite scout can contact more than pro_contact_limit players.
        for player_id in 1u64..=5u64 {
            client.pay_to_contact(&scout, &player_id);
        }
        assert_eq!(client.get_scout_contacts(&scout).len(), 5);
    }

    #[test]
    fn test_pro_contact_limit_resets_on_renewal() {
        // After a Pro scout renews, the contact counter must reset to 0.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);
        let period_secs: u64 = 30 * 24 * 60 * 60;
        let fees = FeeConfig {
            contact_fee_stroops: 100_000,
            basic_sub_stroops: 1_000_000,
            pro_sub_stroops: 3_000_000,
            elite_sub_stroops: 7_000_000,
            sub_duration_secs: period_secs,
            pro_contact_limit: 2,
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        };
        client.initialize(&admin, &xlm, &fees);

        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 200_000_000);
        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Exhaust the limit in period 1.
        client.pay_to_contact(&scout, &1u64);
        client.pay_to_contact(&scout, &2u64);
        assert!(client.try_pay_to_contact(&scout, &3u64).is_err());

        // Advance ledger past subscription + MIN_UPGRADE_INTERVAL so renewal is allowed.
        env.ledger().with_mut(|l| {
            l.timestamp += period_secs + 3_601;
        });

        // Renew subscription.
        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Counter should be reset — scout can contact new players again.
        client.pay_to_contact(&scout, &3u64);
        client.pay_to_contact(&scout, &4u64);
        assert_eq!(
            client.try_pay_to_contact(&scout, &5u64),
            Err(Ok(ScoutAccessError::ContactQuotaExceeded))
        );
    }

    #[test]
    fn test_validate_fee_config_rejects_zero_pro_contact_limit() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);

        let bad_fees = FeeConfig {
            contact_fee_stroops: 100_000,
            basic_sub_stroops: 1_000_000,
            pro_sub_stroops: 3_000_000,
            elite_sub_stroops: 7_000_000,
            sub_duration_secs: 30 * 24 * 60 * 60,
            pro_contact_limit: 0, // invalid — must be > 0
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        };
        let res = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(res, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_log_trial_offer_elite() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        let idx = client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(idx, 1);
        assert_eq!(client.get_trial_count(&1u64), 1);

        let offer = client.get_trial_offer(&1u64, &1u32);
        assert_eq!(offer.player_id, 1);
        assert_eq!(offer.scout, scout);
    }

    // #795: expire_trial_offers must sweep only escrows past their
    // expires_at, refunding the scout and removing the record, while
    // leaving still-live escrows untouched.
    #[test]
    fn test_expire_trial_offers_sweeps_only_expired_escrows() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);

        // Two trial offers logged now — both will be past expires_at
        // (default_fees: trial_offer_expiry_secs = 3_600) after the jump below.
        client.pay_to_contact(&scout, &1u64);
        client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        client.pay_to_contact(&scout, &2u64);
        client.log_trial_offer(
            &scout,
            &2u64,
            &String::from_str(&env, "QmPK2s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        // A third offer, logged 30 minutes later, will still be well within
        // its 1h expiry window when the sweep runs.
        env.ledger().with_mut(|l| l.timestamp += 1_800);
        client.pay_to_contact(&scout, &3u64);
        client.log_trial_offer(
            &scout,
            &3u64,
            &String::from_str(&env, "QmPK3s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        // Advance past the 1h window for offers 1 & 2 (3_700s old) but not
        // for offer 3 (only 1_900s old).
        env.ledger().with_mut(|l| l.timestamp += 1_900);

        let escrow_amount = default_fees().trial_offer_escrow_stroops;
        let token = TokenClient::new(&env, &xlm);
        let balance_before = token.balance(&scout);

        let swept = client.expire_trial_offers(&10u32);
        assert_eq!(swept, 2, "only the two expired escrows should be swept");
        assert_eq!(token.balance(&scout), balance_before + escrow_amount * 2);

        // Swept escrows are gone: confirming them now hits the "already
        // gone" path, since the TrialEscrow record no longer exists.
        let player_wallet = Address::generate(&env);
        let res1 = client.try_confirm_trial_offer(&player_wallet, &1u64, &1u32, &None);
        assert_eq!(res1, Err(Ok(ScoutAccessError::TrialOfferAlreadyConfirmed)));
        let res2 = client.try_confirm_trial_offer(&player_wallet, &2u64, &1u32, &None);
        assert_eq!(res2, Err(Ok(ScoutAccessError::TrialOfferAlreadyConfirmed)));

        // A second sweep finds nothing new to expire — offer 3 is not due yet.
        assert_eq!(client.expire_trial_offers(&10u32), 0);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #15)")]
    fn test_log_trial_offer_rejects_empty_hash() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.log_trial_offer(&scout, &1u64, &String::from_str(&env, ""));
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #15)")]
    fn test_log_trial_offer_rejects_short_hash() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.log_trial_offer(&scout, &1u64, &String::from_str(&env, "Q"));
    }

    #[test]
    fn test_log_trial_offer_accepts_cidv0() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        let idx = client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(idx, 1);
        assert_eq!(client.get_trial_count(&1u64), 1);
    }

    #[test]
    fn test_log_trial_offer_accepts_cidv1() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        let idx = client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(
                &env,
                "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
            ),
        );
        assert_eq!(idx, 1);
        assert_eq!(client.get_trial_count(&1u64), 1);
    }

    #[test]
    fn test_trial_offer_ttl_extended_after_ledger_advance() {
        let (env, admin, xlm, _contract_id, client) = setup();

        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000;
            l.min_persistent_entry_ttl = 500;
            l.max_entry_ttl = 600_000;
        });

        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);

        client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000 + 1_000;
        });

        let offer = client.get_trial_offer(&1u64, &1u32);
        assert_eq!(offer.player_id, 1);
        assert_eq!(client.get_trial_count(&1u64), 1);
    }

    /// Issue: TrialCounter's TTL must be extended alongside TrialOffer in the
    /// same log_trial_offer call, so a counter expiry never causes the next
    /// offer to be written back to index 1 (overwriting/orphaning the first
    /// offer that is still live).
    #[test]
    fn test_trial_counter_survives_ttl_expiry_and_continues_incrementing() {
        let (env, admin, xlm, _contract_id, client) = setup();

        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000;
            l.min_persistent_entry_ttl = 500;
            l.max_entry_ttl = 600_000;
        });

        let scout1 = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout1, 100_000_000);
        client.subscribe(&scout1, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout1, &1u64);

        let idx1 = client.log_trial_offer(
            &scout1,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(idx1, 1);

        // Advance past the default persistent entry TTL (500). Without the
        // TrialCounter extend_ttl call, this would drop the counter back to
        // 0 and the next offer would be written to index 1 again, silently
        // overwriting the offer above.
        // Advance the ledger well past the default persistent entry TTL (500)
        // that a not-yet-extended TrialCounter would have expired at.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000 + 1_000;
        });

        // A second scout logs an offer for the same player. TrialCounter is
        // keyed by player_id only, so this exercises the shared counter
        // without depending on scout1's unrelated Subscription/ContactRecord
        // TTL window.
        // TrialCounter must have survived the expiry window untouched.
        assert_eq!(client.get_trial_count(&1u64), 1);

        // A second scout logs an offer for the same player at the new ledger
        // sequence. TrialCounter is keyed by player_id only, so this exercises
        // the shared counter without depending on scout1's now-expired
        // Subscription/ContactRecord entries (an unrelated TTL window).
        let scout2 = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout2, 100_000_000);
        client.subscribe(&scout2, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout2, &1u64);

        let idx2 = client.log_trial_offer(
            &scout2,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        // The new offer must land at index 2, not collide with index 1.
        assert_eq!(idx2, 2);
        assert_eq!(client.get_trial_count(&1u64), 2);

        // Neither offer is orphaned: both remain readable at their original
        // indices with their original scout.
        // The counter must continue from 1, not reset (i.e. become 2), and
        // the original offer at index 1 must remain intact.
        assert_eq!(idx2, 2);
        assert_eq!(client.get_trial_count(&1u64), 2);
        let offer1 = client.get_trial_offer(&1u64, &1u32);
        assert_eq!(offer1.scout, scout1);
        let offer2 = client.get_trial_offer(&1u64, &2u32);
        assert_eq!(offer2.scout, scout2);
    }

    /// Issue: a scout whose Elite subscription has expired must not be able to
    /// log a trial offer. Verifies that `try_log_trial_offer` returns
    /// `Err(Ok(ScoutAccessError::SubscriptionExpired))` once the ledger
    /// timestamp is advanced past `expires_at`, and that no trial offer is
    /// stored after the rejected call.
    #[test]
    fn test_log_trial_offer_rejected_after_subscription_expires() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);

        // Fund the scout and subscribe to Elite tier.
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);

        // Confirm the subscription was recorded correctly.
        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Elite);
        assert!(sub.expires_at > sub.subscribed_at);

        // Advance the ledger timestamp one second past the subscription expiry.
        env.ledger().with_mut(|l| {
            l.timestamp = sub.expires_at + 1;
        });

        // try_log_trial_offer must return SubscriptionExpired.
        let player_id: u64 = 1;
        let details_hash = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB");
        let result = client.try_log_trial_offer(&scout, &player_id, &details_hash);
        assert_eq!(result, Err(Ok(ScoutAccessError::SubscriptionExpired)));

        // No trial offer must have been stored after the rejected call.
        assert_eq!(client.get_trial_count(&player_id), 0);
    }

    #[test]
    fn test_admin_transfer_propose_replace_and_accept() {
        let (env, old_admin, _xlm, contract_id, client) = setup();
        let stale_admin = Address::generate(&env);
        let new_admin = Address::generate(&env);

        client.propose_admin(&stale_admin);
        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, events::ADMIN_TRANSFER_PROPOSED),
                        old_admin.clone(),
                    )
                        .into_val(&env),
                    stale_admin.clone().into_val(&env),
                )
            ]
        );

        client.pause_contract();
        client.unpause_contract();

        client.propose_admin(&new_admin);
        env.as_contract(&contract_id, || {
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::Admin),
                Some(old_admin.clone())
            );
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::PendingAdmin),
                Some(new_admin.clone())
            );
        });

        env.mock_auths(&[MockAuth {
            address: &new_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.accept_admin();
        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, events::ADMIN_TRANSFERRED),
                        old_admin.clone(),
                    )
                        .into_val(&env),
                    new_admin.clone().into_val(&env),
                )
            ]
        );
        env.as_contract(&contract_id, || {
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::Admin),
                Some(new_admin)
            );
            assert!(!env.storage().persistent().has(&DataKey::PendingAdmin));
        });
    }

    #[test]
    #[should_panic]
    fn test_third_party_cannot_accept_admin() {
        let (env, _old_admin, _xlm, contract_id, client) = setup();
        let pending_admin = Address::generate(&env);
        let third_party = Address::generate(&env);
        client.propose_admin(&pending_admin);

        env.mock_auths(&[MockAuth {
            address: &third_party,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.accept_admin();
    }

    #[test]
    #[should_panic]
    fn test_subscription_expiry() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Fast-forward past expiry (31 days)
        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });

        // Should panic with SubscriptionExpired
        client.pay_to_contact(&scout, &1u64);
    }

    #[test]
    fn test_upgrade_preserves_admin() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);
        client.subscribe(&scout, &SubscriptionTier::Basic);

        let new_wasm_hash = env
            .deployer()
            .upload_contract_wasm(soroban_sdk::Bytes::new(&env));
        client.upgrade(&new_wasm_hash);

        // Admin persisted — admin-gated call still works
        client.pause_contract();
        // Subscription data persisted
        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Basic);
    }

    #[test]
    fn test_pause_unpause_events() {
        let (env, admin, _, _, client) = setup();

        client.pause_contract();
        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, crate::events::CONTRACT_PAUSED),
                        admin.clone()
                    )
                        .into_val(&env),
                    ().into_val(&env)
                )
            ]
        );

        client.unpause_contract();
        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, crate::events::CONTRACT_UNPAUSED),
                        admin.clone()
                    )
                        .into_val(&env),
                    ().into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_full_scout_workflow() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        let player_id = 1u64;
        let details_hash = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB");

        let fees = default_fees();

        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &player_id);
        client.log_trial_offer(&scout, &player_id, &details_hash);

        assert!(client.has_contacted(&scout, &player_id));
        assert_eq!(client.get_trial_count(&player_id), 1);

        let expected_fees = fees.elite_sub_stroops + fees.contact_fee_stroops;
        assert_eq!(client.get_accumulated_fees(), expected_fees);

        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Elite);

        let offer = client.get_trial_offer(&player_id, &1u32);
        assert_eq!(offer.scout, scout);
        assert_eq!(offer.player_id, player_id);
        assert_eq!(offer.details_hash, details_hash);
        assert!(sub.expires_at > sub.subscribed_at);
    }

    #[test]
    fn test_withdraw_fees_success() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);

        client.subscribe(&scout, &SubscriptionTier::Basic);
        assert_eq!(client.get_accumulated_fees(), 1_000_000);

        let recipient = Address::generate(&env);
        let withdrawn = client.withdraw_fees(&recipient);
        assert_eq!(withdrawn, 1_000_000);
        assert_eq!(client.get_accumulated_fees(), 0);

        let token_client = TokenClient::new(&env, &xlm);
        assert_eq!(token_client.balance(&recipient), 1_000_000);
    }

    #[test]
    fn test_withdraw_fees_insufficient() {
        let (env, _admin, _xlm, _contract_id, client) = setup();
        let recipient = Address::generate(&env);
        let result = client.try_withdraw_fees(&recipient);
        assert_eq!(result, Err(Ok(ScoutAccessError::NoFeesToWithdraw)));
    }

    #[test]
    fn test_fee_accumulation_overflow() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&DataKey::AccumulatedFees, &(i128::MAX - 1));
        });

        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        assert_eq!(result, Err(Ok(ScoutAccessError::Overflow)));
    }

    // -------------------------------------------------------------------------
    // validate_fee_config tests
    // -------------------------------------------------------------------------

    fn make_contract() -> (Env, Address, Address, ScoutAccessContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);
        (env, admin, xlm, client)
    }

    #[test]
    fn test_initialize_zero_contact_fee_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            contact_fee_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_basic_sub_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            basic_sub_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_pro_sub_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            pro_sub_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_elite_sub_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            elite_sub_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_sub_duration_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            sub_duration_secs: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_negative_fee_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            contact_fee_stroops: -1,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_valid_fee_config_succeeds() {
        let (_env, admin, xlm, client) = make_contract();
        let result = client.try_initialize(&admin, &xlm, &default_fees());
        assert!(result.is_ok());
    }

    #[test]
    fn test_update_fee_config_zero_subscription_fee_returns_invalid_input() {
        let (_, _, _, _, client) = setup();
        let bad_fees = FeeConfig {
            basic_sub_stroops: 0,
            ..default_fees()
        };
        let result = client.try_update_fee_config(&bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_update_fee_config_zero_contact_fee_returns_invalid_input() {
        let (_, _, _, _, client) = setup();
        let bad_fees = FeeConfig {
            contact_fee_stroops: 0,
            ..default_fees()
        };
        let result = client.try_update_fee_config(&bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_update_fee_config_zero_duration_returns_invalid_input() {
        let (_, _, _, _, client) = setup();
        let bad_fees = FeeConfig {
            sub_duration_secs: 0,
            ..default_fees()
        };
        let result = client.try_update_fee_config(&bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_update_fee_config_valid_succeeds() {
        let (_, _, _, _, client) = setup();
        let new_fees = FeeConfig {
            contact_fee_stroops: 200_000,
            basic_sub_stroops: 2_000_000,
            pro_sub_stroops: 5_000_000,
            elite_sub_stroops: 10_000_000,
            sub_duration_secs: 60 * 24 * 60 * 60,
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
            pro_contact_limit: 15,
        };
        let result = client.try_update_fee_config(&new_fees);
        assert!(result.is_ok());
        let stored = client.get_fee_config();
        assert_eq!(stored.contact_fee_stroops, 200_000);
    }

    // -------------------------------------------------------------------------
    // Issue #822: FeeConfig validation for trial-offer escrow fields
    // -------------------------------------------------------------------------

    #[test]
    fn test_initialize_zero_trial_escrow_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            trial_offer_escrow_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_trial_expiry_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            trial_offer_expiry_secs: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_negative_trial_escrow_returns_invalid_input() {
        let (_env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            trial_offer_escrow_stroops: -1,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_update_fee_config_zero_trial_escrow_returns_invalid_input() {
        let (_, _, _, _, client) = setup();
        let bad_fees = FeeConfig {
            trial_offer_escrow_stroops: 0,
            ..default_fees()
        };
        let result = client.try_update_fee_config(&bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_update_fee_config_zero_trial_expiry_returns_invalid_input() {
        let (_, _, _, _, client) = setup();
        let bad_fees = FeeConfig {
            trial_offer_expiry_secs: 0,
            ..default_fees()
        };
        let result = client.try_update_fee_config(&bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    // -------------------------------------------------------------------------
    // Downgrade guard tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_downgrade_elite_to_pro_before_expiry_returns_error() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);

        let result = client.try_subscribe(&scout, &SubscriptionTier::Pro);
        assert_eq!(
            result,
            Err(Ok(ScoutAccessError::SubscriptionDowngradeNotAllowed))
        );
    }

    #[test]
    fn test_downgrade_elite_to_basic_before_expiry_returns_error() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);

        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        assert_eq!(
            result,
            Err(Ok(ScoutAccessError::SubscriptionDowngradeNotAllowed))
        );
    }

    #[test]
    fn test_downgrade_pro_to_basic_before_expiry_returns_error() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        assert_eq!(
            result,
            Err(Ok(ScoutAccessError::SubscriptionDowngradeNotAllowed))
        );
    }

    #[test]
    fn test_upgrade_basic_to_elite_before_expiry_succeeds() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Basic);
        let basic_sub = client.get_subscription(&scout);

        // Advance past the minimum interval to allow the upgrade
        env.ledger().with_mut(|l| {
            l.timestamp += MIN_UPGRADE_INTERVAL_SECS + 1;
        });

        client.subscribe(&scout, &SubscriptionTier::Elite);
        let elite_sub = client.get_subscription(&scout);

        assert_eq!(elite_sub.tier, SubscriptionTier::Elite);
        assert!(elite_sub.expires_at >= basic_sub.expires_at);
    }

    #[test]
    fn test_upgrade_pro_to_elite_before_expiry_succeeds() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Advance past the minimum interval to allow the upgrade
        env.ledger().with_mut(|l| {
            l.timestamp += MIN_UPGRADE_INTERVAL_SECS + 1;
        });

        client.subscribe(&scout, &SubscriptionTier::Elite);

        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Elite);
    }

    #[test]
    fn test_resubscribe_at_lower_tier_after_expiry_succeeds() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);

        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });

        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        assert!(result.is_ok());
        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Basic);
    }

    #[test]
    fn test_resubscribe_same_tier_after_expiry_succeeds() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });

        let result = client.try_subscribe(&scout, &SubscriptionTier::Pro);
        assert!(result.is_ok());
    }

    // -------------------------------------------------------------------------
    // Upgrade timing guard tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_rapid_upgrade_rejected() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        // Subscribe to Basic
        client.subscribe(&scout, &SubscriptionTier::Basic);

        // Attempt upgrade to Elite immediately — should be rejected
        let result = client.try_subscribe(&scout, &SubscriptionTier::Elite);
        assert_eq!(result, Err(Ok(ScoutAccessError::UpgradeTooSoon)));
    }

    #[test]
    fn test_rapid_same_tier_renewal_succeeds() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Same-tier renewals should not be blocked by the upgrade timing guard.
        let result = client.try_subscribe(&scout, &SubscriptionTier::Pro);
        assert!(result.is_ok());
    }

    #[test]
    fn test_upgrade_after_interval_succeeds() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Basic);

        // Advance time past the minimum interval
        env.ledger().with_mut(|l| {
            l.timestamp += MIN_UPGRADE_INTERVAL_SECS + 1;
        });

        // Upgrade should now succeed
        let result = client.try_subscribe(&scout, &SubscriptionTier::Elite);
        assert!(result.is_ok());
        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Elite);
    }

    // -------------------------------------------------------------------------
    // refund_subscription tests
    // -------------------------------------------------------------------------

    // -------------------------------------------------------------------------
    // Fee accumulation tests across multiple subscriptions
    // -------------------------------------------------------------------------
    #[test]
    fn test_accumulated_fees_sum_across_multiple_scout_subscriptions() {
        let (env, admin, xlm, _contract_id, client) = setup();

        // Create three scouts and mint tokens for each
        let scout_basic = Address::generate(&env);
        let scout_pro = Address::generate(&env);
        let scout_elite = Address::generate(&env);

        let fees = default_fees();

        mint_token(&env, &xlm, &admin, &scout_basic, 10_000_000);
        mint_token(&env, &xlm, &admin, &scout_pro, 10_000_000);
        mint_token(&env, &xlm, &admin, &scout_elite, 20_000_000);

        // Subscribe each scout to a different tier
        client.subscribe(&scout_basic, &SubscriptionTier::Basic);
        client.subscribe(&scout_pro, &SubscriptionTier::Pro);
        client.subscribe(&scout_elite, &SubscriptionTier::Elite);

        // Verify accumulated fees equals sum of all three subscription fees
        let expected_total = fees.basic_sub_stroops + fees.pro_sub_stroops + fees.elite_sub_stroops;
        assert_eq!(client.get_accumulated_fees(), expected_total);

        // Withdraw fees and verify the amount
        let recipient = Address::generate(&env);
        let withdrawn = client.withdraw_fees(&recipient);
        assert_eq!(withdrawn, expected_total);

        // Verify accumulated fees reset to 0
        assert_eq!(client.get_accumulated_fees(), 0);

        // Verify token balances are consistent
        let token_client = TokenClient::new(&env, &xlm);
        assert_eq!(token_client.balance(&recipient), expected_total);
    }

    // -------------------------------------------------------------------------
    // pause_contract event tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_pause_contract_emits_contract_paused_event() {
        let (env, admin, _xlm, contract_id, client) = setup();

        // Pause the contract
        client.pause_contract();

        // Verify the contract_paused event is emitted with correct topic and admin payload
        let events = env.events().all();
        assert_eq!(
            events.filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "contract_paused"), admin.clone()).into_val(&env),
                    ().into_val(&env)
                )
            ]
        );

        // Verify contract is actually paused
        assert!(client.health().paused);
    }

    #[test]
    fn test_refund_subscription_success() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);

        let contract_balance_before = TokenClient::new(&env, &xlm).balance(&client.address);
        let scout_balance_before = TokenClient::new(&env, &xlm).balance(&scout);

        let refund_amount = 1_000_000i128;
        client.refund_subscription(&scout, &refund_amount);

        let contract_balance_after = TokenClient::new(&env, &xlm).balance(&client.address);
        let scout_balance_after = TokenClient::new(&env, &xlm).balance(&scout);

        assert_eq!(
            contract_balance_before - refund_amount,
            contract_balance_after
        );
        assert_eq!(scout_balance_before + refund_amount, scout_balance_after);
    }

    #[test]
    fn test_refund_subscription_zero_amount_rejected() {
        let (env, _admin, _xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        let result = client.try_refund_subscription(&scout, &0i128);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_refund_subscription_negative_amount_rejected() {
        let (env, _admin, _xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        let result = client.try_refund_subscription(&scout, &(-1i128));
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_refund_subscription_exceeds_balance_returns_insufficient_fee() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 1_000_000);
        // Scout subscribes Basic (1_000_000 stroops) — contract now holds 1_000_000
        client.subscribe(&scout, &SubscriptionTier::Basic);
        // Attempt to refund more than the contract balance
        let result = client.try_refund_subscription(&scout, &2_000_000i128);
        assert_eq!(result, Err(Ok(ScoutAccessError::InsufficientFee)));
    }

    #[test]
    fn test_refund_subscription_within_balance_succeeds() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);
        client.subscribe(&scout, &SubscriptionTier::Basic);
        // Refund exactly what was paid — within balance
        let result = client.try_refund_subscription(&scout, &1_000_000i128);
        assert!(result.is_ok());
    }

    // -------------------------------------------------------------------------
    // #451: set_progress_contract emits progress_contract_updated event
    // -------------------------------------------------------------------------

    #[test]
    fn test_set_progress_contract_emits_event() {
        let (env, _admin, _xlm, contract_id, client) = setup();
        let progress_addr = Address::generate(&env);

        client.set_progress_contract(&progress_addr);

        // set_progress_contract now emits both the legacy
        // progress_contract_updated event and the new wiring_updated event
        // (issue #1041) on every successful call.
        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, crate::events::PROGRESS_CONTRACT_UPDATED),
                        _admin.clone(),
                    )
                        .into_val(&env),
                    progress_addr.clone().into_val(&env),
                ),
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, crate::events::WIRING_UPDATED),
                        _admin.clone(),
                        Symbol::new(&env, "progress_contract"),
                    )
                        .into_val(&env),
                    (progress_addr.clone(), 1u32).into_val(&env),
                )
            ]
        );
    }

    #[test]
    fn test_update_progress_contract_is_alias_for_set() {
        let (env, _admin, _xlm, contract_id, client) = setup();
        let first = Address::generate(&env);
        let second = Address::generate(&env);

        client.set_progress_contract(&first);
        client.update_progress_contract(&second);

        env.as_contract(&contract_id, || {
            assert_eq!(
                env.storage()
                    .instance()
                    .get::<DataKey, Address>(&DataKey::ProgressContract),
                Some(second)
            );
        });
    }

    // -------------------------------------------------------------------------
    // Wiring observability (issue #1041)
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_progress_contract_before_and_after_configuration() {
        let (env, _admin, _xlm, _contract_id, client) = setup();

        assert_eq!(client.get_progress_contract(), None);

        let progress_addr = Address::generate(&env);
        client.set_progress_contract(&progress_addr);
        assert_eq!(client.get_progress_contract(), Some(progress_addr));
    }

    #[test]
    fn test_get_wiring_state_initially_unconfigured() {
        let (_, _admin, _xlm, _contract_id, client) = setup();

        let state = client.get_wiring_state();
        assert_eq!(state.progress_contract.address, None);
        assert_eq!(state.progress_contract.epoch, 0);
        assert_eq!(state.registration_contract.address, None);
        assert_eq!(state.registration_contract.epoch, 0);
        assert!(!state.is_fully_wired());
    }

    #[test]
    fn test_get_wiring_state_reflects_both_links_and_bumps_epoch() {
        let (env, _admin, _xlm, _contract_id, client) = setup();

        let progress_addr = Address::generate(&env);
        let reg_addr = Address::generate(&env);
        client.set_progress_contract(&progress_addr);
        client.set_registration_contract(&reg_addr);

        let state = client.get_wiring_state();
        assert_eq!(state.progress_contract.address, Some(progress_addr));
        assert_eq!(state.progress_contract.epoch, 1);
        assert_eq!(state.registration_contract.address, Some(reg_addr));
        assert_eq!(state.registration_contract.epoch, 1);
        assert!(state.is_fully_wired());

        // Freely re-settable — no first-call-only guard on either link — and
        // a second call must bump the epoch again, not reset it.
        let new_progress_addr = Address::generate(&env);
        client.set_progress_contract(&new_progress_addr);
        assert_eq!(
            client.get_wiring_state().progress_contract.epoch,
            2,
            "re-wiring the same link must bump its epoch again"
        );
    }

    #[test]
    fn test_set_registration_contract_emits_wiring_updated_event() {
        let (env, _admin, _xlm, contract_id, client) = setup();
        let reg_addr = Address::generate(&env);

        client.set_registration_contract(&reg_addr);

        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, crate::events::REGISTRATION_CONTRACT_UPDATED),
                        _admin.clone(),
                    )
                        .into_val(&env),
                    reg_addr.clone().into_val(&env),
                ),
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, crate::events::WIRING_UPDATED),
                        _admin.clone(),
                        Symbol::new(&env, "registration_contract"),
                    )
                        .into_val(&env),
                    (reg_addr, 1u32).into_val(&env),
                )
            ]
        );
    }

    // -------------------------------------------------------------------------
    // Integration test: log_trial_offer advances player to EliteTier via the
    // real progress contract cross-contract call.
    // -------------------------------------------------------------------------

    #[test]
    fn test_log_trial_offer_advances_player_to_elite_tier() {
        use scoutchain_progress::ProgressContract;
        use scoutchain_progress::ProgressContractClient;
        use scoutchain_shared_types::ProgressLevel;
        use scoutchain_verification::VerificationContract;
        use scoutchain_verification::VerificationContractClient;

        let env = Env::default();
        env.mock_all_auths();

        // --- deploy verification contract ---
        let ver_id = env.register(VerificationContract, ());
        let ver_client = VerificationContractClient::new(&env, &ver_id);
        let ver_admin = Address::generate(&env);
        ver_client.initialize(&ver_admin);

        // --- deploy progress contract ---
        let progress_id = env.register(ProgressContract, ());
        let progress_client = ProgressContractClient::new(&env, &progress_id);
        let progress_admin = Address::generate(&env);
        progress_client.initialize(&progress_admin);
        progress_client.set_verification_contract(&ver_id);

        // --- deploy scout_access contract ---
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let scout_access_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &scout_access_id);
        client.initialize(&admin, &xlm, &default_fees());

        // Wire scout_access → progress
        client.set_progress_contract(&progress_id);
        // Whitelist scout_access as the secondary caller of advance_level.
        progress_client.set_scout_access_contract(&scout_access_id);

        // Pre-advance the player to PerformanceMilestones (Level 2) via the
        // verification contract (the whitelisted primary caller) so that
        // log_trial_offer can push them to EliteTier (Level 3).
        let player_id = 1u64;
        progress_client.advance_level(&ver_id, &player_id, &1u32); // → VerifiedIdentity
        progress_client.advance_level(&ver_id, &player_id, &2u32); // → PerformanceMilestones
        assert_eq!(
            progress_client.get_level(&player_id),
            ProgressLevel::PerformanceMilestones
        );

        // Register a milestone so the trial-offer's milestone_ref (index 1)
        // validates against the verification contract's milestone count.
        let validator = Address::generate(&env);
        ver_client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        ver_client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "scored"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
            &None,
        );

        // Scout subscribes at Elite tier and logs a trial offer
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &player_id);
        let idx = client.log_trial_offer(
            &scout,
            &player_id,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(idx, 1);

        // log_trial_offer is step 1 of the two-step flow: it escrows funds but
        // does NOT advance the level. The player must confirm the offer to
        // release the escrow and advance to EliteTier (Level 3).
        let player_wallet = Address::generate(&env);
        client.confirm_trial_offer(&player_wallet, &player_id, &idx, &None);

        // Player must now be at EliteTier
        assert_eq!(
            progress_client.get_level(&player_id),
            ProgressLevel::EliteTier
        );
    }

    #[test]
    fn test_log_trial_offer_already_at_max_level_does_not_fail() {
        use scoutchain_progress::ProgressContract;
        use scoutchain_progress::ProgressContractClient;
        use scoutchain_shared_types::ProgressLevel;
        use scoutchain_verification::VerificationContract;
        use scoutchain_verification::VerificationContractClient;

        let env = Env::default();
        env.mock_all_auths();

        // --- deploy verification contract ---
        let ver_id = env.register(VerificationContract, ());
        let ver_client = VerificationContractClient::new(&env, &ver_id);
        let ver_admin = Address::generate(&env);
        ver_client.initialize(&ver_admin);

        // --- deploy progress contract ---
        let progress_id = env.register(ProgressContract, ());
        let progress_client = ProgressContractClient::new(&env, &progress_id);
        let progress_admin = Address::generate(&env);
        progress_client.initialize(&progress_admin);
        progress_client.set_verification_contract(&ver_id);

        // --- deploy scout_access contract ---
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let scout_access_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &scout_access_id);
        client.initialize(&admin, &xlm, &default_fees());

        // Wire scout_access → progress
        client.set_progress_contract(&progress_id);
        // Whitelist scout_access as the secondary caller of advance_level.
        progress_client.set_scout_access_contract(&scout_access_id);

        // Pre-advance the player all the way to EliteTier via the
        // verification contract (the whitelisted primary caller).
        let player_id = 2u64;
        progress_client.advance_level(&ver_id, &player_id, &1u32); // → VerifiedIdentity
        progress_client.advance_level(&ver_id, &player_id, &2u32); // → PerformanceMilestones
        progress_client.advance_level(&ver_id, &player_id, &3u32); // → EliteTier
        assert_eq!(
            progress_client.get_level(&player_id),
            ProgressLevel::EliteTier
        );

        // Register a milestone so the trial-offer's milestone_ref (index 1)
        // validates against the verification contract's milestone count.
        let validator = Address::generate(&env);
        ver_client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        ver_client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "scored"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
            &None,
        );

        // log_trial_offer must still succeed even though AlreadyAtMaxLevel is returned
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &player_id);
        let result = client.try_log_trial_offer(
            &scout,
            &player_id,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert!(
            result.is_ok(),
            "AlreadyAtMaxLevel must not fail the trial offer"
        );

        // Player stays at EliteTier
        assert_eq!(
            progress_client.get_level(&player_id),
            ProgressLevel::EliteTier
        );
    }

    // -------------------------------------------------------------------------
    // #454: Missing XlmToken key returns typed NotInitialized error
    // -------------------------------------------------------------------------

    #[test]
    fn test_subscribe_missing_xlm_token_returns_not_initialized() {
        let (env, _admin, _xlm, contract_id, client) = setup();
        // Remove the XlmToken key from instance storage to simulate expiry/absence.
        env.as_contract(&contract_id, || {
            env.storage().instance().remove(&DataKey::XlmToken);
        });
        let scout = Address::generate(&env);
        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        assert_eq!(result, Err(Ok(ScoutAccessError::NotInitialized)));
    }

    // -------------------------------------------------------------------------
    // #456: Per-(scout, player) trial offer rate limit
    // -------------------------------------------------------------------------

    #[test]
    fn test_second_trial_offer_within_cooldown_is_rejected() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        // First offer — must succeed.
        client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        // Second offer to the same player within the 24-hour cooldown — must fail.
        let result = client.try_log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(result, Err(Ok(ScoutAccessError::TrialOfferRateLimited)));
    }

    #[test]
    fn test_trial_offer_allowed_after_cooldown_expires() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        // Advance past the 24-hour cooldown.
        env.ledger().with_mut(|l| {
            l.timestamp += TRIAL_OFFER_COOLDOWN_SECS + 1;
        });

        let result = client.try_log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_trial_offer_to_different_player_not_rate_limited() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        client.pay_to_contact(&scout, &2u64);

        // Offer to a DIFFERENT player must not be rate-limited.
        let result = client.try_log_trial_offer(
            &scout,
            &2u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert!(result.is_ok());
    }

    // -------------------------------------------------------------------------
    // #424: Pause contract blocks log_trial_offer
    // -------------------------------------------------------------------------

    #[test]
    fn test_log_trial_offer_when_contract_paused_returns_contract_paused() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        let player_id = 1u64;
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        // Subscribe scout to Elite tier
        client.subscribe(&scout, &SubscriptionTier::Elite);

        // Pause the contract
        client.pause_contract();

        // Attempt to log trial offer while paused — should be rejected
        let result = client.try_log_trial_offer(
            &scout,
            &player_id,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));

        // Verify no trial offer record was written
        assert_eq!(client.get_trial_count(&player_id), 0);
    }

    #[test]
    fn test_log_trial_offer_succeeds_after_unpause() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        let player_id = 1u64;
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        // Subscribe scout to Elite tier
        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &player_id);

        // Pause the contract
        client.pause_contract();

        // Attempt to log trial offer while paused — should fail
        let paused_result = client.try_log_trial_offer(
            &scout,
            &player_id,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(paused_result, Err(Ok(ScoutAccessError::ContractPaused)));

        // Unpause the contract
        client.unpause_contract();

        // Same call should now succeed
        let result = client.try_log_trial_offer(
            &scout,
            &player_id,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert!(result.is_ok());
        assert_eq!(client.get_trial_count(&player_id), 1);
    }

    // -------------------------------------------------------------------------
    // #462: subscription_created / subscription_renewed events
    // -------------------------------------------------------------------------

    #[test]
    fn test_subscription_created_event_emitted_on_first_subscribe() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);

        // subscription_created at index 0, followed by the legacy scout_subscribed
        // event. Checked immediately — `events().all()` only reflects the most
        // recent invocation, and `get_subscription` below is itself a separate
        // invocation.
        let emitted = env.events().all().filter_by_contract(&contract_id);
        let sub = client.get_subscription(&scout);
        assert_eq!(
            emitted,
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "subscription_created"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Elite, sub.subscribed_at, sub.expires_at).into_val(&env)
                ),
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "scout_subscribed"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Elite, default_fees().elite_sub_stroops).into_val(&env)
                )
            ]
        );
        // Event includes scout, tier, and expiry (acceptance criteria #462)
        assert_eq!(sub.tier, SubscriptionTier::Elite);
        assert!(sub.expires_at > sub.subscribed_at);
    }

    #[test]
    fn test_subscription_renewed_event_emitted_on_renewal() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        // First subscription
        client.subscribe(&scout, &SubscriptionTier::Basic);

        // Advance past the minimum upgrade interval and beyond expiry
        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60; // 31 days — subscription expired
        });

        // Renew
        client.subscribe(&scout, &SubscriptionTier::Basic);

        // The renewal call is the last invocation, so `events().all()` returns
        // exactly its two events: subscription_renewed then legacy scout_subscribed.
        // `get_subscription` below is itself a separate invocation, so it must
        // be called after the events check.
        let emitted = env.events().all().filter_by_contract(&contract_id);
        let sub = client.get_subscription(&scout);
        assert_eq!(
            emitted,
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "subscription_renewed"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Basic, sub.subscribed_at, sub.expires_at).into_val(&env)
                ),
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "scout_subscribed"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Basic, default_fees().basic_sub_stroops).into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_subscription_event_payload_includes_scout_tier_and_expiry() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Verify the subscription_created event payload contains tier + timestamps,
        // followed by the legacy scout_subscribed event. Checked immediately —
        // `events().all()` only reflects the most recent invocation, and
        // `get_subscription` below is itself a separate invocation.
        let emitted = env.events().all().filter_by_contract(&contract_id);
        let sub = client.get_subscription(&scout);
        assert_eq!(
            emitted,
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "subscription_created"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Pro, sub.subscribed_at, sub.expires_at).into_val(&env)
                ),
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "scout_subscribed"), scout.clone()).into_val(&env),
                    (SubscriptionTier::Pro, default_fees().pro_sub_stroops).into_val(&env)
                )
            ]
        );
    }

    // -------------------------------------------------------------------------
    // Fee reconciliation: mixed-operation stress test
    //
    // Verifies that get_accumulated_fees() stays in exact sync with a manually
    // maintained expected total across a mixed sequence of real-world operations,
    // including the batch_contact_players skip/no-charge path for already-contacted
    // players.  Also validates withdraw_fees() zeroes the accumulator.
    // -------------------------------------------------------------------------

    // =========================================================================
    // Check-precedence tests
    //
    // Each test sets up a scenario where TWO OR MORE error conditions are
    // simultaneously true and asserts the *specific* error that the documented
    // check-precedence order requires.  These tests are the executable
    // contract for the numbered tables in docs/CONTRACT_REFERENCE.md.
    // =========================================================================

    // -------------------------------------------------------------------------
    // subscribe — precedence tests
    // -------------------------------------------------------------------------

    /// ContractPaused beats NotInitialized (Priority 1 > Priority 2).
    /// When the contract is paused the caller should see ContractPaused even if
    /// the contract has also somehow lost its Initialized flag.
    #[test]
    fn test_subscribe_paused_beats_not_initialized() {
        let (env, _admin, _xlm, contract_id, client) = setup();
        // Wipe the Initialized key to make NotInitialized also true.
        env.as_contract(&contract_id, || {
            env.storage().instance().remove(&DataKey::Initialized);
        });
        // Pause the contract (writes Paused = true even without Initialized).
        env.as_contract(&contract_id, || {
            env.storage().instance().set(&DataKey::Paused, &true);
        });

        let scout = Address::generate(&env);
        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        // Priority 1: ContractPaused wins over Priority 2: NotInitialized.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ContractPaused beats SubscriptionDowngradeNotAllowed (Priority 1 > Priority 4).
    /// A scout with an active Elite subscription attempts to downgrade to Basic
    /// while the contract is paused.  They must see ContractPaused, not
    /// SubscriptionDowngradeNotAllowed.
    #[test]
    fn test_subscribe_paused_beats_downgrade_error() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        // Subscribe to Elite first.
        client.subscribe(&scout, &SubscriptionTier::Elite);

        // Now pause the contract.
        client.pause_contract();

        // Attempt downgrade while paused — two conditions simultaneously true:
        //   (a) ContractPaused
        //   (b) SubscriptionDowngradeNotAllowed (active Elite → trying Basic)
        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        // Priority 1: ContractPaused wins.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ContractPaused beats UpgradeTooSoon (Priority 1 > Priority 5).
    /// A scout upgrades within the 1-hour window while the contract is paused.
    #[test]
    fn test_subscribe_paused_beats_upgrade_too_soon() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Basic);
        // Pause immediately (no time advance — UpgradeTooSoon is also true).
        client.pause_contract();

        let result = client.try_subscribe(&scout, &SubscriptionTier::Elite);
        // Priority 1: ContractPaused wins over Priority 5: UpgradeTooSoon.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// SubscriptionDowngradeNotAllowed beats UpgradeTooSoon (Priority 4 > 5)
    /// when both are simultaneously true (active sub + downgrade + within
    /// 1-hour upgrade interval).
    #[test]
    fn test_subscribe_downgrade_beats_upgrade_too_soon() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        // No time advance — UpgradeTooSoon is also in effect.
        // But the requested tier is a downgrade, so downgrade check fires first.
        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        // Priority 4: SubscriptionDowngradeNotAllowed wins.
        assert_eq!(
            result,
            Err(Ok(ScoutAccessError::SubscriptionDowngradeNotAllowed))
        );
    }

    // -------------------------------------------------------------------------
    // #245 — Subscription downgrade guard edge cases
    // -------------------------------------------------------------------------

    /// A first-time subscriber with no prior subscription record must never
    /// encounter the downgrade guard. The guard only activates when an
    /// existing subscription is found in persistent storage. A new scout
    /// subscribing to Basic, Pro, or Elite must always succeed regardless of
    /// tier rank.
    #[test]
    fn test_first_time_subscriber_any_tier_bypasses_downgrade_guard() {
        let (env, admin, xlm, _contract_id, client) = setup();

        let scout_basic = Address::generate(&env);
        let scout_pro = Address::generate(&env);
        let scout_elite = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout_basic, 10_000_000);
        mint_token(&env, &xlm, &admin, &scout_pro, 10_000_000);
        mint_token(&env, &xlm, &admin, &scout_elite, 10_000_000);

        // All three must succeed — no prior subscription means no guard.
        assert!(client
            .try_subscribe(&scout_basic, &SubscriptionTier::Basic)
            .is_ok());
        assert!(client
            .try_subscribe(&scout_pro, &SubscriptionTier::Pro)
            .is_ok());
        assert!(client
            .try_subscribe(&scout_elite, &SubscriptionTier::Elite)
            .is_ok());
    }

    /// A same-tier re-subscribe while the subscription is still active is not
    /// a downgrade (tier_rank is equal, not less). The downgrade guard must
    /// not fire. However, the UpgradeTooSoon guard (minimum 1-hour interval)
    /// still applies for same-tier renewals — callers must wait at least
    /// MIN_UPGRADE_INTERVAL_SECS before the same-tier renewal is accepted.
    /// This test verifies the behaviour after the interval has elapsed.
    #[test]
    fn test_same_tier_resubscribe_before_expiry_after_interval_is_not_a_downgrade() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Advance past the minimum upgrade interval but stay well within the
        // 30-day subscription window.
        env.ledger().with_mut(|l| {
            l.timestamp += MIN_UPGRADE_INTERVAL_SECS + 1;
        });

        // A same-tier subscribe must succeed: tier_rank(Pro) == tier_rank(Pro)
        // does not satisfy the `< existing.tier_rank` condition, so the
        // downgrade guard is not triggered.
        let result = client.try_subscribe(&scout, &SubscriptionTier::Pro);
        assert!(
            result.is_ok(),
            "same-tier re-subscribe after interval must not return SubscriptionDowngradeNotAllowed"
        );
        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Pro);
    }

    /// At the exact expiry timestamp (now == expires_at) the condition
    /// `now <= existing.expires_at` is true, so the downgrade guard still
    /// applies. A downgrade attempted at exactly the expiry ledger timestamp
    /// must return SubscriptionDowngradeNotAllowed, not succeed silently.
    ///
    /// This is intentional: the subscription is considered active up to and
    /// including its final second. Callers must wait for a timestamp strictly
    /// greater than expires_at before attempting a lower-tier subscription.
    ///
    /// This behaviour is documented in CONTRACT_REFERENCE.md § subscribe
    /// (Downgrade guard boundary).
    #[test]
    fn test_downgrade_blocked_at_exact_expiry_timestamp() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);

        // Retrieve the subscription so we know the exact expires_at value.
        let sub = client.get_subscription(&scout);
        let expires_at = sub.expires_at;

        // Set ledger timestamp to exactly expires_at — the boundary value.
        env.ledger().with_mut(|l| {
            l.timestamp = expires_at;
        });

        // At exactly expires_at the subscription is still considered active
        // (now <= expires_at), so a downgrade must be blocked.
        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        assert_eq!(
            result,
            Err(Ok(ScoutAccessError::SubscriptionDowngradeNotAllowed)),
            "downgrade at exactly expires_at must still be blocked (now <= expires_at is true)"
        );

        // One second later (now > expires_at) the subscription is expired
        // and the downgrade must succeed.
        env.ledger().with_mut(|l| {
            l.timestamp = expires_at + 1;
        });
        let result_after = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        assert!(
            result_after.is_ok(),
            "downgrade one second after expires_at must succeed (subscription is expired)"
        );
    }

    /// Verify that tier_rank comparison is ordinal, not name-based. Pro has
    /// rank 2 and Basic has rank 1, so Pro→Basic is a downgrade (rank 2 > 1).
    /// This test makes the rank ordering explicit and guards against any future
    /// refactoring that reorders the SubscriptionTier enum variants.
    #[test]
    fn test_downgrade_pro_rank_to_basic_rank_is_blocked_while_active() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // No time advance — subscription is active; tier_rank(Basic)=1 < tier_rank(Pro)=2.
        let result = client.try_subscribe(&scout, &SubscriptionTier::Basic);
        assert_eq!(
            result,
            Err(Ok(ScoutAccessError::SubscriptionDowngradeNotAllowed)),
            "Pro(rank=2) → Basic(rank=1) must be blocked while the Pro subscription is active"
        );
    }

    // -------------------------------------------------------------------------
    // pay_to_contact — precedence tests
    // -------------------------------------------------------------------------

    /// ContractPaused beats ScoutNotSubscribed (Priority 1 > Priority 4).
    /// When the contract is paused AND the scout has no subscription, the
    /// caller must see ContractPaused.
    #[test]
    fn test_pay_to_contact_paused_beats_not_subscribed() {
        let (env, _admin, _xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        // Scout has no subscription (ScoutNotSubscribed also true).
        client.pause_contract();

        let result = client.try_pay_to_contact(&scout, &1u64);
        // Priority 1: ContractPaused wins over Priority 4: ScoutNotSubscribed.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ContractPaused beats SubscriptionExpired (Priority 1 > Priority 5).
    #[test]
    fn test_pay_to_contact_paused_beats_expired() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Expire the subscription, then pause.
        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });
        client.pause_contract();

        let result = client.try_pay_to_contact(&scout, &1u64);
        // Priority 1: ContractPaused wins over Priority 5: SubscriptionExpired.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ContractPaused beats AlreadyContacted (Priority 1 > Priority 6).
    #[test]
    fn test_pay_to_contact_paused_beats_already_contacted() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        // Now contact record exists AND we pause.
        client.pause_contract();

        let result = client.try_pay_to_contact(&scout, &1u64);
        // Priority 1: ContractPaused wins over Priority 6: AlreadyContacted.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ScoutNotSubscribed beats SubscriptionExpired — impossible in practice
    /// because expired subscriptions still have a record. Confirmed: Priority 4
    /// (no record) is structurally exclusive from Priority 5 (record exists but
    /// expired). Test documents the order by checking that an expired
    /// subscription surfaces as SubscriptionExpired (Priority 5), not
    /// ScoutNotSubscribed.
    #[test]
    fn test_pay_to_contact_expired_sub_returns_expired_not_not_subscribed() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);
        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });

        let result = client.try_pay_to_contact(&scout, &99u64);
        // Priority 5 fires: SubscriptionExpired (not ScoutNotSubscribed).
        assert_eq!(result, Err(Ok(ScoutAccessError::SubscriptionExpired)));
    }

    /// SubscriptionExpired beats AlreadyContacted (Priority 5 > Priority 6).
    /// When the subscription is expired AND a ContactRecord already exists, the
    /// caller must see SubscriptionExpired — the expiry check runs before the
    /// duplicate-contact guard in the source.
    #[test]
    fn test_pay_to_contact_expired_beats_already_contacted() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);

        // Expire the subscription.
        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });

        let result = client.try_pay_to_contact(&scout, &1u64);
        // Priority 5: SubscriptionExpired wins over Priority 6: AlreadyContacted.
        assert_eq!(result, Err(Ok(ScoutAccessError::SubscriptionExpired)));
    }

    // -------------------------------------------------------------------------
    // batch_contact_players — precedence tests
    // -------------------------------------------------------------------------

    /// ContractPaused beats ScoutNotSubscribed (Priority 1 > Priority 4).
    #[test]
    fn test_batch_contact_paused_beats_not_subscribed() {
        let (env, _admin, _xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        // No subscription, contract paused.
        client.pause_contract();

        let mut batch = soroban_sdk::Vec::new(&env);
        batch.push_back(1u64);
        let result = client.try_batch_contact_players(&scout, &batch);
        // Priority 1: ContractPaused wins.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ContractPaused beats SubscriptionExpired (Priority 1 > Priority 4).
    #[test]
    fn test_batch_contact_paused_beats_expired() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);
        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });
        client.pause_contract();

        let mut batch = soroban_sdk::Vec::new(&env);
        batch.push_back(1u64);
        let result = client.try_batch_contact_players(&scout, &batch);
        // Priority 1: ContractPaused wins.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ScoutNotSubscribed / SubscriptionExpired beat ContactQuotaExceeded
    /// (Priority 4 > Priority 5).  A Pro scout at their monthly limit whose
    /// subscription has also expired must see SubscriptionExpired, not quota.
    #[test]
    fn test_batch_contact_expired_beats_quota_exceeded() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let contract_id = env.register(ScoutAccessContract, ());
        let client = ScoutAccessContractClient::new(&env, &contract_id);
        let fees = FeeConfig {
            pro_contact_limit: 1,
            ..default_fees()
        };
        client.initialize(&admin, &xlm, &fees);

        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Exhaust the quota.
        let mut single = soroban_sdk::Vec::new(&env);
        single.push_back(1u64);
        client.batch_contact_players(&scout, &single);

        // Expire the subscription.
        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });

        // New batch — quota is exceeded AND subscription expired.
        let mut batch = soroban_sdk::Vec::new(&env);
        batch.push_back(2u64);
        let result = client.try_batch_contact_players(&scout, &batch);
        // Priority 4: SubscriptionExpired wins over Priority 5: ContactQuotaExceeded.
        assert_eq!(result, Err(Ok(ScoutAccessError::SubscriptionExpired)));
    }

    // -------------------------------------------------------------------------
    // log_trial_offer — precedence tests
    // -------------------------------------------------------------------------

    /// ContractPaused beats InvalidInput (Priority 1 > Priority 3).
    /// Paused contract with a malformed CID must return ContractPaused.
    #[test]
    fn test_log_trial_offer_paused_beats_invalid_input() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pause_contract();

        // details_hash is invalid (too short) — InvalidInput also true.
        let result = client.try_log_trial_offer(&scout, &1u64, &String::from_str(&env, "bad"));
        // Priority 1: ContractPaused wins over Priority 3: InvalidInput.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ContractPaused beats ScoutNotSubscribed (Priority 1 > Priority 4).
    #[test]
    fn test_log_trial_offer_paused_beats_not_subscribed() {
        let (env, _admin, _xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        client.pause_contract();

        let result = client.try_log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        // Priority 1: ContractPaused wins over Priority 4: ScoutNotSubscribed.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// ContractPaused beats Unauthorized-tier (Priority 1 > Priority 5).
    /// Paused contract, Pro scout (not Elite), valid CID → ContractPaused.
    #[test]
    fn test_log_trial_offer_paused_beats_unauthorized_tier() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);
        client.pause_contract();

        let result = client.try_log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        // Priority 1: ContractPaused wins over Priority 5: Unauthorized.
        assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    }

    /// InvalidInput beats ScoutNotSubscribed (Priority 3 > Priority 4).
    /// A scout with no subscription and an invalid CID must see InvalidInput.
    #[test]
    fn test_log_trial_offer_invalid_input_beats_not_subscribed() {
        let (env, _admin, _xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        // No subscription — ScoutNotSubscribed is also true.

        let result = client.try_log_trial_offer(&scout, &1u64, &String::from_str(&env, "tooshort"));
        // Priority 3: InvalidInput wins over Priority 4: ScoutNotSubscribed.
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    /// InvalidInput beats Unauthorized-tier (Priority 3 > Priority 5).
    /// A Pro scout (non-Elite) supplying an invalid CID must see InvalidInput.
    #[test]
    fn test_log_trial_offer_invalid_input_beats_unauthorized_tier() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);
        // Pro tier (non-Elite) — Unauthorized also true.

        let result = client.try_log_trial_offer(&scout, &1u64, &String::from_str(&env, "tooshort"));
        // Priority 3: InvalidInput wins over Priority 5: Unauthorized.
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    /// SubscriptionExpired beats Unauthorized-tier (Priority 4 > Priority 5).
    /// A Pro scout whose subscription has also expired must see SubscriptionExpired.
    #[test]
    fn test_log_trial_offer_expired_beats_unauthorized_tier() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);
        // Expire the subscription.
        env.ledger().with_mut(|l| {
            l.timestamp += 31 * 24 * 60 * 60;
        });

        let result = client.try_log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        // Priority 4: SubscriptionExpired wins over Priority 5: Unauthorized.
        assert_eq!(result, Err(Ok(ScoutAccessError::SubscriptionExpired)));
    }

    /// Unauthorized-tier beats Unauthorized-not-contacted (Priority 5 > Priority 6).
    /// An Elite scout who has NOT previously contacted the player must see
    /// Unauthorized (from the tier check) when both conditions are simultaneously
    /// active — but in this test the scout IS Elite so only Priority 6 fires.
    /// This test verifies that the "not-contacted" check returns Unauthorized
    /// ONLY when tier is correct (Elite), confirming Priority 5 fires before 6.
    #[test]
    fn test_log_trial_offer_unauthorized_tier_beats_unauthorized_not_contacted() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        // Pro tier — tier check fires first (Priority 5).
        client.subscribe(&scout, &SubscriptionTier::Pro);
        // Also has NOT contacted the player — Priority 6 condition also true.

        let result = client.try_log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        // Priority 5 fires: Unauthorized (non-Elite tier).
        // Same error code as Priority 6, but we confirm it fires before contact-check
        // by verifying the error even when the contact record would also be absent.
        assert_eq!(result, Err(Ok(ScoutAccessError::Unauthorized)));
    }

    /// Unauthorized-not-contacted beats TrialOfferRateLimited (Priority 6 > Priority 7).
    /// An Elite scout who has contacted one player, sent an offer within the last
    /// 24 h to that player, but then tries to log an offer for a DIFFERENT player
    /// they have NEVER contacted — they must see Unauthorized (no contact record),
    /// not TrialOfferRateLimited.
    #[test]
    fn test_log_trial_offer_not_contacted_beats_rate_limited() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        // Contact and offer player 1 (rate-limit is now in effect for player 1).
        client.pay_to_contact(&scout, &1u64);
        client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        // Player 2: scout has NOT contacted them (Priority 6 condition true).
        // For player 1 rate-limit would fire (Priority 7), but we are testing
        // a different player here to isolate the not-contacted path.
        let result = client.try_log_trial_offer(
            &scout,
            &2u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        // Priority 6: Unauthorized (not contacted player 2) wins.
        assert_eq!(result, Err(Ok(ScoutAccessError::Unauthorized)));
    }

    #[test]
    fn test_fee_reconciliation_mixed_sequence() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let fees = default_fees();

        // Scout A subscribes Basic (Tier 1); Scout B subscribes Pro (Tier 2).
        // Both need enough XLM for their subscription plus subsequent contacts.
        // Scout A: 1 sub + 2 contacts (individual + batch new player) = 1_000_000 + 200_000
        // Scout B: 1 sub + 1 contact = 3_000_000 + 100_000
        let scout_a = Address::generate(&env);
        let scout_b = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout_a, 5_000_000);
        mint_token(&env, &xlm, &admin, &scout_b, 5_000_000);

        // ----------------------------------------------------------------
        // Step 1 — Subscriptions
        // ----------------------------------------------------------------
        client.subscribe(&scout_a, &SubscriptionTier::Basic);
        client.subscribe(&scout_b, &SubscriptionTier::Pro);

        let mut expected_fees: i128 = fees.basic_sub_stroops + fees.pro_sub_stroops;
        // 1_000_000 + 3_000_000 = 4_000_000
        assert_eq!(
            client.get_accumulated_fees(),
            expected_fees,
            "fees after subscriptions"
        );

        // ----------------------------------------------------------------
        // Step 2 — Individual contacts
        // ----------------------------------------------------------------
        // Player IDs used throughout the test.
        let player_1: u64 = 1;
        let player_2: u64 = 2;
        let player_3: u64 = 3;

        client.pay_to_contact(&scout_a, &player_1);
        expected_fees += fees.contact_fee_stroops; // +100_000 → 4_100_000

        client.pay_to_contact(&scout_b, &player_2);
        expected_fees += fees.contact_fee_stroops; // +100_000 → 4_200_000

        assert_eq!(
            client.get_accumulated_fees(),
            expected_fees,
            "fees after individual contacts"
        );

        // ----------------------------------------------------------------
        // Step 3 — Batch contact: Player 1 (already contacted, skip/no-charge)
        //          + Player 3 (new, charged)
        // ----------------------------------------------------------------
        let mut batch = soroban_sdk::Vec::new(&env);
        batch.push_back(player_1); // already contacted by scout_a → must be skipped
        batch.push_back(player_3); // new → charged

        let new_contacts = client.batch_contact_players(&scout_a, &batch);

        // Exactly one new contact should have been recorded (Player 3 only).
        assert_eq!(
            new_contacts, 1,
            "batch_contact_players must skip already-contacted Player 1 and only charge for Player 3"
        );

        // Only one contact fee should have been charged, not two.
        expected_fees += fees.contact_fee_stroops; // +100_000 → 4_300_000

        // ----------------------------------------------------------------
        // Reconciliation Check #1 — accumulated fees must exactly match
        // the independently maintained expected total.
        // ----------------------------------------------------------------
        // Expected breakdown:
        //   Basic subscription (Scout A)  :   1_000_000
        //   Pro subscription (Scout B)    :   3_000_000
        //   Scout A contacts Player 1     :     100_000
        //   Scout B contacts Player 2     :     100_000
        //   Scout A batch, Player 3 only  :     100_000
        //                                  -----------
        //   Total                          :   4_300_000
        assert_eq!(
            client.get_accumulated_fees(),
            expected_fees,
            "Reconciliation Check #1 failed: accumulated fees do not match expected total. \
             If this assertion fails it likely means batch_contact_players is charging for \
             already-contacted (skipped) players — check the first-pass loop in lib.rs."
        );
        assert_eq!(
            expected_fees, 4_300_000,
            "sanity check: expected_fees constant cross-check"
        );

        // ----------------------------------------------------------------
        // Step 4 — Withdrawal & Reset Check
        // ----------------------------------------------------------------
        let pre_withdrawal_total = client.get_accumulated_fees();
        let recipient = Address::generate(&env);
        let withdrawn = client.withdraw_fees(&recipient);

        // Returned amount must equal the pre-withdrawal total.
        assert_eq!(
            withdrawn, pre_withdrawal_total,
            "withdraw_fees must return the full accumulated total"
        );

        // Accumulator must drop to strictly 0 after withdrawal.
        assert_eq!(
            client.get_accumulated_fees(),
            0,
            "accumulated fees must be exactly 0 after withdraw_fees"
        );

        // Token balance of the recipient must equal what was withdrawn.
        let token_client = TokenClient::new(&env, &xlm);
        assert_eq!(
            token_client.balance(&recipient),
            withdrawn,
            "recipient token balance must equal withdrawn amount"
        );
    }

    // -------------------------------------------------------------------------
    // get_fee_config_history tests (#849)
    // -------------------------------------------------------------------------

    /// History is empty before any update_fee_config call.
    #[test]
    fn test_fee_config_history_empty_before_any_update() {
        let (_env, _admin, _xlm, _contract_id, client) = setup();
        let history = client.get_fee_config_history();
        assert_eq!(history.len(), 0);
    }

    /// Each update_fee_config call appends the old config to history, oldest-first.
    #[test]
    fn test_fee_config_history_records_previous_configs_oldest_first() {
        let (_env, _admin, _xlm, _contract_id, client) = setup();

        let fees_v2 = FeeConfig {
            contact_fee_stroops: 200_000,
            basic_sub_stroops: 2_000_000,
            pro_sub_stroops: 5_000_000,
            elite_sub_stroops: 10_000_000,
            sub_duration_secs: 30 * 24 * 60 * 60,
            pro_contact_limit: 15,
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        };
        let fees_v3 = FeeConfig {
            contact_fee_stroops: 300_000,
            basic_sub_stroops: 3_000_000,
            pro_sub_stroops: 6_000_000,
            elite_sub_stroops: 11_000_000,
            sub_duration_secs: 30 * 24 * 60 * 60,
            pro_contact_limit: 20,
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        };

        // First update: moves v1 (default) → history[0], sets v2 as current.
        client.update_fee_config(&fees_v2);
        let history = client.get_fee_config_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history.get(0).unwrap().config.contact_fee_stroops, 100_000);

        // Second update: moves v2 → history[1], sets v3 as current.
        client.update_fee_config(&fees_v3);
        let history = client.get_fee_config_history();
        assert_eq!(history.len(), 2);
        // Oldest-first: history[0] is v1, history[1] is v2.
        assert_eq!(history.get(0).unwrap().config.contact_fee_stroops, 100_000);
        assert_eq!(history.get(1).unwrap().config.contact_fee_stroops, 200_000);

        // Current config should be v3.
        assert_eq!(client.get_fee_config().contact_fee_stroops, 300_000);
    }

    /// After 5 updates the cap (5 entries) is reached; the 6th update evicts
    /// the oldest entry so the list never exceeds 5 items.
    #[test]
    fn test_fee_config_history_capped_at_five_entries() {
        let (_env, _admin, _xlm, _contract_id, client) = setup();

        // Perform 6 updates; each one bumps the contact_fee by 100_000.
        for i in 1u32..=6 {
            let new_fees = FeeConfig {
                contact_fee_stroops: 100_000 * (i as i128 + 1),
                basic_sub_stroops: 1_000_000,
                pro_sub_stroops: 3_000_000,
                elite_sub_stroops: 7_000_000,
                sub_duration_secs: 30 * 24 * 60 * 60,
                pro_contact_limit: 10,
                trial_offer_escrow_stroops: 500_000,
                trial_offer_expiry_secs: 3_600,
            };
            client.update_fee_config(&new_fees);
        }

        // History must be capped at 5 even though 6 updates were made.
        let history = client.get_fee_config_history();
        assert_eq!(history.len(), 5);

        // The very first config (contact_fee=100_000) was evicted.
        // The oldest retained entry is the config set on the 2nd update
        // (contact_fee=200_000), which had contact_fee_stroops=100_000*2.
        assert_eq!(history.get(0).unwrap().config.contact_fee_stroops, 200_000);
    }

    // -------------------------------------------------------------------------
    // #861: set_auto_renew / renew_if_due
    // -------------------------------------------------------------------------

    #[test]
    fn test_set_auto_renew_stores_flag_and_emits_event() {
        let (env, _admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &_admin, &scout, 10_000_000);

        // Opt in.
        client.set_auto_renew(&scout, &true);

        // env.events().all() only returns events from the most recent contract
        // invocation, so it must be read immediately after the write — a later
        // view call (e.g. get_auto_renew) would clear the buffer.
        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (
                        Symbol::new(&env, crate::events::AUTO_RENEW_SET),
                        scout.clone()
                    )
                        .into_val(&env),
                    true.into_val(&env),
                )
            ]
        );
        assert!(client.get_auto_renew(&scout));

        // Opt back out.
        client.set_auto_renew(&scout, &false);
        assert!(!client.get_auto_renew(&scout));
    }

    #[test]
    fn test_renew_if_due_noop_outside_window() {
        let (env, _admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &_admin, &scout, 100_000_000);

        // Subscribe and enable auto-renewal.
        client.subscribe(&scout, &SubscriptionTier::Basic);
        client.set_auto_renew(&scout, &true);

        // The subscription was just created — we are nowhere near the renewal
        // window.  renew_if_due should return Ok and do nothing (no charge).
        let before = client.get_subscription(&scout);
        client.renew_if_due(&scout);
        let after = client.get_subscription(&scout);
        assert_eq!(before.expires_at, after.expires_at);
    }

    #[test]
    fn test_renew_if_due_renews_inside_window() {
        use soroban_sdk::testutils::Ledger;

        let (env, _admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &_admin, &scout, 100_000_000);

        let sub_duration: u64 = 30 * 24 * 60 * 60; // 30 days in seconds

        env.ledger().with_mut(|l| l.timestamp = 1_000_000);
        client.subscribe(&scout, &SubscriptionTier::Basic);
        client.set_auto_renew(&scout, &true);

        let before = client.get_subscription(&scout);

        // Jump to 96 % of the way through the subscription (inside the last-10%-window).
        let renewal_ts = before.expires_at - sub_duration / 20; // ~95 % in
        env.ledger().with_mut(|l| l.timestamp = renewal_ts);

        client.renew_if_due(&scout);

        let after = client.get_subscription(&scout);
        // A new subscription was written; expires_at must advance.
        assert!(after.expires_at > before.expires_at);
        assert_eq!(after.subscribed_at, renewal_ts);
    }

    #[test]
    fn test_renew_if_due_renews_after_expiry() {
        use soroban_sdk::testutils::Ledger;

        let (env, _admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &_admin, &scout, 100_000_000);

        let sub_duration: u64 = 30 * 24 * 60 * 60;

        env.ledger().with_mut(|l| l.timestamp = 1_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.set_auto_renew(&scout, &true);

        let before = client.get_subscription(&scout);

        // Jump past expiry.
        env.ledger()
            .with_mut(|l| l.timestamp = before.expires_at + 100);

        client.renew_if_due(&scout);

        let after = client.get_subscription(&scout);
        assert!(after.expires_at > before.expires_at);
        // expires_at should be roughly now + sub_duration.
        let expected_expiry = before.expires_at + 100 + sub_duration;
        assert_eq!(after.expires_at, expected_expiry);
    }

    #[test]
    fn test_renew_if_due_fails_when_not_enabled() {
        let (env, _admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &_admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Basic);
        // Deliberately do NOT call set_auto_renew.

        let result = client.try_renew_if_due(&scout);
        assert_eq!(result, Err(Ok(ScoutAccessError::AutoRenewNotEnabled)));
    }

    #[test]
    fn test_renew_if_due_fails_with_no_subscription() {
        let (env, _admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &_admin, &scout, 10_000_000);

        client.set_auto_renew(&scout, &true);

        let result = client.try_renew_if_due(&scout);
        assert_eq!(result, Err(Ok(ScoutAccessError::ScoutNotSubscribed)));
    }

    /// Regression test for the TrialEscrow TTL bug.
    ///
    /// Before the fix, `log_trial_offer` wrote `DataKey::TrialEscrow` via
    /// `.set()` but never called `extend_ttl` on it, leaving it with Soroban's
    /// default minimal persistent TTL (~4,096 ledgers ≈ 5.7 hours). This is
    /// shorter than `trial_offer_expiry_secs` (1 hour in the test default
    /// config), but more importantly it is shorter than the platform's 30-day
    /// activity cycle, meaning the record could be silently archived before
    /// either `confirm_trial_offer` or `expire_trial_offers` ran — locking
    /// the escrowed XLM with no normal read path to resolve it.
    ///
    /// After the fix, `log_trial_offer` extends the TTL to `TRIAL_TTL_EXTEND_TO`
    /// (518,400 ledgers ≈ 30 days), well beyond the default. This test:
    /// 1. Calls `log_trial_offer` and asserts the TTL is immediately set to the
    ///    full policy value (> 5,000 ledgers, far above any reasonable default).
    /// 2. Advances the ledger sequence past the old default TTL (~4,096 ledgers)
    ///    and asserts the `TrialEscrow` entry is still present (not archived).
    #[test]
    fn test_trial_escrow_ttl_extended_on_log_trial_offer() {
        use soroban_sdk::testutils::{storage::Persistent, Ledger};

        let (env, _admin, xlm, _contract_id, client) = setup();

        // Start at a known sequence with headroom for max_entry_ttl.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000;
            l.min_persistent_entry_ttl = 500;
            l.max_entry_ttl = 600_000;
        });

        let scout = Address::generate(&env);
        // Fund with enough XLM for Elite subscription + escrow.
        mint_token(&env, &xlm, &_admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);

        let index = client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(index, 1);

        // --- Assertion 1: TTL is immediately extended to the policy value ---
        // The write-side fix calls extend_ttl(..., TRIAL_TTL_THRESHOLD=259_200,
        // TRIAL_TTL_EXTEND_TO=518_400), so the stored TTL must be substantially
        // larger than the minimal default (~500 in this test environment).
        env.as_contract(&client.address, || {
            let ttl = env
                .storage()
                .persistent()
                .get_ttl(&DataKey::TrialEscrow(1u64, index));
            assert!(
                ttl > 5_000,
                "TrialEscrow TTL should be extended to policy value after log_trial_offer, got {}",
                ttl
            );
        });

        // --- Assertion 2: Entry survives past the old default persistent TTL ---
        // Without the fix, the default TTL would be 500 ledgers (our test env
        // min_persistent_entry_ttl). Advance well past that.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000 + 2_000; // 4× the default test TTL
        });

        // The TrialEscrow entry must still be accessible — not archived.
        env.as_contract(&client.address, || {
            assert!(
                env.storage()
                    .persistent()
                    .has(&DataKey::TrialEscrow(1u64, index)),
                "TrialEscrow must still be present after advancing past the old default TTL"
            );
        });
    }
}

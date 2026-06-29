#![cfg_attr(target_family = "wasm", no_std)]mod errors;
mod events;
mod types;

use errors::ScoutAccessError;
use types::{DataKey, Subscription, TrialOffer};
pub use types::{FeeConfig, SubscriptionTier};

use soroban_sdk::{contract, contractimpl, token, Address, Env, String};

use scoutchain_shared_types::{validate_cid, ContractHealth};

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

// Instance TTL bump
const INSTANCE_TTL_MIN: u32 = 100;
const INSTANCE_TTL_MAX: u32 = 500;

// Persistent storage TTL bump for subscriptions / contact records.
const PERSISTENT_TTL_MIN: u32 = 200;
const PERSISTENT_TTL_MAX: u32 = 2_000;

// Trial offer TTL: ~30 days at 5 s/ledger.
const TRIAL_TTL_THRESHOLD: u32 = 259_200;
const TRIAL_TTL_EXTEND_TO: u32 = 518_400;
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Minimum interval (seconds) between subscribe calls for the same scout
// to prevent race conditions / double-charging on rapid upgrades.
const MIN_UPGRADE_INTERVAL_SECS: u64 = 3600;

// #456: Minimum cooldown (seconds) between trial offers from the same scout
// to the same player — enforces one pending offer per (scout, player) per day.
const TRIAL_OFFER_COOLDOWN_SECS: u64 = 86_400; // 24 hours

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
        Self::bump_instance_ttl(&env);
        if env.storage().instance().has(&DataKey::Initialized) {
            return Err(ScoutAccessError::AlreadyInitialized);
        }
        Self::validate_fee_config(&fee_config)?;
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(&DataKey::Admin, ADMIN_BUMP_LEDGERS, ADMIN_BUMP_LEDGERS);
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
        Self::require_admin(&env)?;
        Self::validate_fee_config(&fee_config)?;
        env.storage()
            .instance()
            .set(&DataKey::FeeConfig, &fee_config);
        Ok(())
    }

    pub fn withdraw_fees(env: Env, to: Address) -> Result<i128, ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_admin(&env)?;
        let fees: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedFees)
            .unwrap_or(0i128);
        if fees == 0 {
            return Err(ScoutAccessError::NoFeesToWithdraw);
        }
        let xlm = Self::get_token(&env)?;
        let contract_addr = env.current_contract_address();
        token::Client::new(&env, &xlm).transfer(&contract_addr, &to, &fees);
        env.storage()
            .instance()
            .set(&DataKey::AccumulatedFees, &0i128);
        events::fees_withdrawn(&env, &to, fees);
        Ok(fees)
    }

    pub fn pause_contract(env: Env) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_admin(&env)?;
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(ScoutAccessError::NotInitialized)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        events::contract_paused(&env, &admin);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_admin(&env)?;
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(ScoutAccessError::NotInitialized)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        events::contract_unpaused(&env, &admin);
        Ok(())
    }

    /// Register the progress contract address so log_trial_offer can
    /// atomically advance the player to Level 3 (admin only).
    pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::ProgressContract, &addr);
        events::progress_contract_updated(&env, &addr);
        Ok(())
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
        Self::require_admin(&env)?;
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
    pub fn upgrade(env: Env, new_wasm_hash: soroban_sdk::BytesN<32>) -> Result<(), ScoutAccessError> {
        Self::require_admin(&env)?;
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
                let min_next = existing
                    .subscribed_at
                    .checked_add(MIN_UPGRADE_INTERVAL_SECS)
                    .ok_or(ScoutAccessError::Overflow)?;
                if now < min_next {
                    return Err(ScoutAccessError::UpgradeTooSoon);
                }
            }
        }

        let config = Self::fee_config(&env);
        let fee = match &tier {
            SubscriptionTier::Basic => config.basic_sub_stroops,
            SubscriptionTier::Pro => config.pro_sub_stroops,
            SubscriptionTier::Elite => config.elite_sub_stroops,
        };

        Self::collect_fee(&env, &scout, fee)?;

        let sub = Subscription {
            scout: scout.clone(),
            tier: tier.clone(),
            expires_at: now
                .checked_add(config.sub_duration_secs)
                .ok_or(ScoutAccessError::Overflow)?,
            subscribed_at: now,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Subscription(scout.clone()), &sub);
        env.storage().persistent().extend_ttl(
            &DataKey::Subscription(scout.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        events::scout_subscribed(&env, &scout, &tier, fee);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Pay-to-contact
    // -------------------------------------------------------------------------

    /// Helper: check Pro tier contact quota. Returns Ok(()) if within limit or not Pro tier.
    fn check_pro_contact_quota(env: &Env, scout: &Address) -> Result<(), ScoutAccessError> {
        let sub: Subscription = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(scout.clone()))
            .ok_or(ScoutAccessError::ScoutNotSubscribed)?;

        // Only Pro tier has a quota
        if sub.tier != SubscriptionTier::Pro {
            return Ok(());
        }

        // Month bucket: use Unix timestamp / seconds per month (30 days)
        const SECONDS_PER_MONTH: u64 = 2_592_000;
        let month_bucket = sub.subscribed_at / SECONDS_PER_MONTH;

        let quota_key = DataKey::ContactCount(scout.clone(), month_bucket);
        let current: u32 = env.storage().persistent().get(&quota_key).unwrap_or(0u32);

        let config = Self::fee_config(&env);
        let limit = config.pro_contact_limit;

        if current >= limit {
            return Err(ScoutAccessError::ContactQuotaExceeded);
        }

        Ok(())
    }

    /// Helper: check Pro tier contact quota with a specific count (batch support).
    fn check_pro_contact_quota_with_count(
        env: &Env,
        scout: &Address,
        requested: u32,
    ) -> Result<(), ScoutAccessError> {
        let sub: Subscription = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(scout.clone()))
            .ok_or(ScoutAccessError::ScoutNotSubscribed)?;

        // Only Pro tier has a quota
        if sub.tier != SubscriptionTier::Pro {
            return Ok(());
        }

        const SECONDS_PER_MONTH: u64 = 2_592_000;
        let month_bucket = sub.subscribed_at / SECONDS_PER_MONTH;

        let quota_key = DataKey::ContactCount(scout.clone(), month_bucket);
        let current: u32 = env.storage().persistent().get(&quota_key).unwrap_or(0u32);

        let config = Self::fee_config(&env);
        let limit = config.pro_contact_limit;

        if current.saturating_add(requested) > limit {
            return Err(ScoutAccessError::ContactQuotaExceeded);
        }

        Ok(())
    }

    /// Helper: increment contact count for Pro tier scouts.
    fn increment_contact_count(env: &Env, scout: &Address) {
        Self::increment_contact_count_by(env, scout, 1)
    }

    /// Helper: increment contact count by N for Pro tier scouts (batch support).
    fn increment_contact_count_by(env: &Env, scout: &Address, count: u32) {
        const SECONDS_PER_MONTH: u64 = 2_592_000;
        let now = env.ledger().timestamp();
        let month_bucket = now / SECONDS_PER_MONTH;

        let quota_key = DataKey::ContactCount(scout.clone(), month_bucket);
        let current: u32 = env.storage().persistent().get(&quota_key).unwrap_or(0u32);
        env.storage()
            .persistent()
            .set(&quota_key, &(current.saturating_add(count)));
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
        scout.require_auth();
        Self::require_active_subscription(&env, &scout)?;
        Self::check_pro_contact_quota(&env, &scout)?;

        let contact_key = DataKey::ContactRecord(player_id, scout.clone());
        if env.storage().persistent().has(&contact_key) {
            return Err(ScoutAccessError::AlreadyContacted);
        }

        let config = Self::fee_config(&env);
        Self::collect_fee(&env, &scout, config.contact_fee_stroops)?;
        Self::increment_contact_count(&env, &scout);

        env.storage().persistent().set(&contact_key, &true);
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
        if !contacted.contains(&player_id) {
            contacted.push_back(player_id);
        }
        env.storage().persistent().set(&index_key, &contacted);
        env.storage()
            .persistent()
            .extend_ttl(&index_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

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
        player_ids: Vec<u64>,
    ) -> Result<u32, ScoutAccessError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        scout.require_auth();
        Self::require_active_subscription(&env, &scout)?;

        let config = Self::fee_config(&env);
        let mut new_contacts: u32 = 0;

        // First pass: count new (uncharged) contacts to compute total fee.
        for i in 0..player_ids.len() {
            let player_id = player_ids.get(i).unwrap();
            if !env
                .storage()
                .persistent()
                .has(&DataKey::ContactRecord(player_id, scout.clone()))
            {
                new_contacts = new_contacts
                    .checked_add(1)
                    .ok_or(ScoutAccessError::Overflow)?;
            }
        }

        if new_contacts == 0 {
            return Ok(0);
        }

        // Check quota with the count we're about to add
        Self::check_pro_contact_quota_with_count(&env, &scout, new_contacts)?;

        // Single token transfer for all new contacts combined.
        let total_fee = config
            .contact_fee_stroops
            .checked_mul(new_contacts as i128)
            .ok_or(ScoutAccessError::Overflow)?;
        Self::collect_fee(&env, &scout, total_fee)?;

        // Second pass: write contact records and emit events.
        for i in 0..player_ids.len() {
            let player_id = player_ids.get(i).unwrap();
            let contact_key = DataKey::ContactRecord(player_id, scout.clone());
            if env.storage().persistent().has(&contact_key) {
                continue;
            }
            env.storage().persistent().set(&contact_key, &true);
            env.storage().persistent().extend_ttl(
                &contact_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
            events::player_contacted(&env, player_id, &scout, config.contact_fee_stroops);
        }

        Self::increment_contact_count_by(&env, &scout, new_contacts);

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

        // #456: Enforce per-(scout, player) cooldown to prevent offer flooding.
        // Reject a second offer from the same scout to the same player within
        // TRIAL_OFFER_COOLDOWN_SECS (24 h). Offers to different players are
        // independent and are not rate-limited against each other.
        let rate_key = DataKey::TrialOfferLastSent(scout.clone(), player_id);
        let now = env.ledger().timestamp();
        if let Some(last_sent) = env
            .storage()
            .persistent()
            .get::<DataKey, u64>(&rate_key)
        {
            let next_allowed = last_sent
                .checked_add(TRIAL_OFFER_COOLDOWN_SECS)
                .ok_or(ScoutAccessError::Overflow)?;
            if now < next_allowed {
                return Err(ScoutAccessError::TrialOfferRateLimited);
            }
        }

        let counter_key = DataKey::TrialCounter(player_id);
        let index: u32 = env.storage().persistent().get(&counter_key).unwrap_or(0u32);
        let next_index = index.checked_add(1).ok_or(ScoutAccessError::Overflow)?;

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
        env.storage().persistent().extend_ttl(
            &rate_key,
            TRIAL_TTL_THRESHOLD,
            TRIAL_TTL_EXTEND_TO,
        );

        // Cross-contract call: advance the player to Level 3 if progress contract is set.
        if let Some(progress_addr) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::ProgressContract)
        {
            let progress_client = progress_contract::Client::new(&env, &progress_addr);
            match progress_client.try_advance_level(&scout, &player_id, &next_index) {
                Ok(_) => {}
                Err(Ok(progress_contract::Error::AlreadyAtMaxLevel)) => {}
                Err(_) => return Err(ScoutAccessError::ProgressCallFailed),
            }
        }

        events::trial_offer_logged(&env, player_id, &scout);
        Ok(next_index)
    }

    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), ScoutAccessError> {
        Self::require_admin(&env)?;
        let old_admin = Self::get_admin(&env);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        events::admin_transferred(&env, &old_admin, &new_admin);
        Ok(())
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

    pub fn get_fee_config(env: Env) -> FeeConfig {
        Self::bump_instance_ttl(&env);
        Self::fee_config(&env)
    }

    pub fn get_accumulated_fees(env: Env) -> i128 {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&DataKey::AccumulatedFees)
            .unwrap_or(0i128)
    }

    pub fn has_contacted(env: Env, scout: Address, player_id: u64) -> bool {
        Self::bump_instance_ttl(&env);
        let key = DataKey::ContactRecord(player_id, scout);
        let exists = env.storage().persistent().has(&key);
        if exists {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        exists
    }

    /// Return all player_ids contacted by `scout` as an O(1) index lookup.
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
        ContractHealth {
            initialized,
            paused,
        }
    }

    /// Returns the deployed crate version (from Cargo.toml at build time).
    pub fn version(env: Env) -> String {
        String::from_str(&env, CONTRACT_VERSION)
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    fn require_admin(env: &Env) -> Result<(), ScoutAccessError> {
    let admin: Address = env
        .storage()
        .persistent()
        .get(&DataKey::Admin)
        .ok_or(ScoutAccessError::NotInitialized)?;

    admin.require_auth();

    env.storage().persistent().extend_ttl(
        &DataKey::Admin,
        ADMIN_BUMP_LEDGERS,
        ADMIN_BUMP_LEDGERS,
    );

    Ok(())
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

    fn get_admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized")
    }

    fn get_token(env: &Env) -> Result<Address, ScoutAccessError> {
        env.storage()
            .instance()
            .get(&DataKey::XlmToken)
            .ok_or(ScoutAccessError::NotInitialized)
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
        let new_total = current
            .checked_add(amount)
            .ok_or(ScoutAccessError::Overflow)?;
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

    /// Validate that every fee field is positive and sub_duration_secs is non-zero.
    fn validate_fee_config(config: &FeeConfig) -> Result<(), ScoutAccessError> {
        if config.contact_fee_stroops <= 0
            || config.basic_sub_stroops <= 0
            || config.pro_sub_stroops <= 0
            || config.elite_sub_stroops <= 0
            || config.sub_duration_secs == 0
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
        let contract_id = env.register_contract(None, ScoutAccessContract);
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
        let contract_id = env.register_contract(None, ScoutAccessContract);
        let client = ScoutAccessContractClient::new(&env, &contract_id);

        client.initialize(&admin, &xlm, &default_fees());

        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "contract_initialized"), admin.clone()).into_val(&env),
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
    fn test_fee_config_updated_event_contains_old_and_new_config() {
        let (env, _admin, _xlm, _contract_id, client) = setup();

        let new_fees = FeeConfig {
            contact_fee_stroops: 200_000,
            basic_sub_stroops: 2_000_000,
            pro_sub_stroops: 5_000_000,
            elite_sub_stroops: 10_000_000,
            sub_duration_secs: 60 * 24 * 60 * 60,
        };

        client.update_fee_config(&new_fees);

        // Storage must reflect the new config.
        let stored = client.get_fee_config();
        assert_eq!(stored.contact_fee_stroops, new_fees.contact_fee_stroops);
    }

    #[test]
    fn test_version() {
        let (env, _, _, _, client) = setup();
        assert_eq!(client.version(), String::from_str(&env, "0.1.0"));
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

        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
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

        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
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

    #[test]
    fn test_player_contacted_event_includes_fee_paid() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &42u64);

        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "player_contacted"), scout.clone()).into_val(&env),
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
    fn test_log_trial_offer_elite() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        let idx = client.log_trial_offer(&scout, &1u64, &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"));
        assert_eq!(idx, 1);
        assert_eq!(client.get_trial_count(&1u64), 1);

        let offer = client.get_trial_offer(&1u64, &1u32);
        assert_eq!(offer.player_id, 1);
        assert_eq!(offer.scout, scout);
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

        client.log_trial_offer(&scout, &1u64, &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"));

        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000 + 1_000;
        });

        let offer = client.get_trial_offer(&1u64, &1u32);
        assert_eq!(offer.player_id, 1);
        assert_eq!(client.get_trial_count(&1u64), 1);
    }

    #[test]
    fn test_transfer_admin_success() {
        let (env, _admin, _xlm, _contract_id, client) = setup();
        let new_admin = Address::generate(&env);

        client.transfer_admin(&new_admin);
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

        let new_wasm_hash = env.deployer().upload_contract_wasm(soroban_sdk::Bytes::new(&env));
        client.upgrade(&new_wasm_hash);

        // Admin persisted — admin-gated call still works
        client.pause_contract();
        // Subscription data persisted
        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Basic);
    }
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
                    (Symbol::new(&env, "contract_paused"),).into_val(&env),
                    admin.clone().into_val(&env)
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
                    (Symbol::new(&env, "contract_unpaused"),).into_val(&env),
                    admin.clone().into_val(&env)
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
        let contract_id = env.register_contract(None, ScoutAccessContract);
        let client = ScoutAccessContractClient::new(&env, &contract_id);
        (env, admin, xlm, client)
    }

    #[test]
    fn test_initialize_zero_contact_fee_returns_invalid_input() {
        let (env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            contact_fee_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_basic_sub_returns_invalid_input() {
        let (env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            basic_sub_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_pro_sub_returns_invalid_input() {
        let (env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            pro_sub_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_elite_sub_returns_invalid_input() {
        let (env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            elite_sub_stroops: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_zero_sub_duration_returns_invalid_input() {
        let (env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            sub_duration_secs: 0,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_negative_fee_returns_invalid_input() {
        let (env, admin, xlm, client) = make_contract();
        let bad_fees = FeeConfig {
            contact_fee_stroops: -1,
            ..default_fees()
        };
        let result = client.try_initialize(&admin, &xlm, &bad_fees);
        assert_eq!(result, Err(Ok(ScoutAccessError::InvalidInput)));
    }

    #[test]
    fn test_initialize_valid_fee_config_succeeds() {
        let (env, admin, xlm, client) = make_contract();
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
        };
        let result = client.try_update_fee_config(&new_fees);
        assert!(result.is_ok());
        let stored = client.get_fee_config();
        assert_eq!(stored.contact_fee_stroops, 200_000);
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
    fn test_rapid_same_tier_renewal_rejected() {
        let (env, admin, xlm, _contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Pro);

        // Attempt same-tier renewal immediately — should be rejected
        let result = client.try_subscribe(&scout, &SubscriptionTier::Pro);
        assert_eq!(result, Err(Ok(ScoutAccessError::UpgradeTooSoon)));
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

assert_eq!(
    scout_balance_before + refund_amount,
    scout_balance_after
);
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

        assert_eq!(
            env.events().all().filter_by_contract(&contract_id),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (Symbol::new(&env, "progress_contract_updated"),).into_val(&env),
                    progress_addr.clone().into_val(&env),
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

        let env = Env::default();
        env.mock_all_auths();

        // --- deploy progress contract ---
        let progress_id = env.register_contract(None, ProgressContract);
        let progress_client = ProgressContractClient::new(&env, &progress_id);
        let progress_admin = Address::generate(&env);
        progress_client.initialize(&progress_admin);

        // --- deploy scout_access contract ---
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let scout_access_id = env.register_contract(None, ScoutAccessContract);
        let client = ScoutAccessContractClient::new(&env, &scout_access_id);
        client.initialize(&admin, &xlm, &default_fees());

        // Wire scout_access → progress
        client.set_progress_contract(&progress_id);

        // Pre-advance the player to PerformanceMilestones (Level 2) so that
        // log_trial_offer can push them to EliteTier (Level 3).
        let player_id = 1u64;
        let caller = Address::generate(&env);
        progress_client.advance_level(&caller, &player_id, &1u32); // → VerifiedIdentity
        progress_client.advance_level(&caller, &player_id, &2u32); // → PerformanceMilestones
        assert_eq!(
            progress_client.get_level(&player_id),
            ProgressLevel::PerformanceMilestones
        );

        // Scout subscribes at Elite tier and logs a trial offer
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);
        let idx = client.log_trial_offer(
            &scout,
            &player_id,
            &String::from_str(&env, "QmTrialOfferIntegration"),
        );
        assert_eq!(idx, 1);

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

        let env = Env::default();
        env.mock_all_auths();

        // --- deploy progress contract ---
        let progress_id = env.register_contract(None, ProgressContract);
        let progress_client = ProgressContractClient::new(&env, &progress_id);
        let progress_admin = Address::generate(&env);
        progress_client.initialize(&progress_admin);

        // --- deploy scout_access contract ---
        let admin = Address::generate(&env);
        let xlm = create_token(&env, &admin);
        let scout_access_id = env.register_contract(None, ScoutAccessContract);
        let client = ScoutAccessContractClient::new(&env, &scout_access_id);
        client.initialize(&admin, &xlm, &default_fees());

        // Wire scout_access → progress
        client.set_progress_contract(&progress_id);

        // Pre-advance the player all the way to EliteTier
        let player_id = 2u64;
        let caller = Address::generate(&env);
        progress_client.advance_level(&caller, &player_id, &1u32); // → VerifiedIdentity
        progress_client.advance_level(&caller, &player_id, &2u32); // → PerformanceMilestones
        progress_client.advance_level(&caller, &player_id, &3u32); // → EliteTier
        assert_eq!(
            progress_client.get_level(&player_id),
            ProgressLevel::EliteTier
        );

        // log_trial_offer must still succeed even though AlreadyAtMaxLevel is returned
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);
        client.subscribe(&scout, &SubscriptionTier::Elite);
        let result = client.try_log_trial_offer(
            &scout,
            &player_id,
            &String::from_str(&env, "QmAlreadyMaxLevel"),
        );
        assert!(result.is_ok(), "AlreadyAtMaxLevel must not fail the trial offer");

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
        let (env, admin, xlm, contract_id, client) = setup();
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
        // First offer — must succeed.
        client.log_trial_offer(&scout, &1u64, &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"));

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
        client.log_trial_offer(&scout, &1u64, &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"));

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
        client.log_trial_offer(&scout, &1u64, &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"));

        // Offer to a DIFFERENT player must not be rate-limited.
        let result = client.try_log_trial_offer(
            &scout,
            &2u64,
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert!(result.is_ok());
    }
}

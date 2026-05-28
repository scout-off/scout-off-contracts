mod errors;
mod events;
mod types;

use errors::ScoutAccessError;
use types::{DataKey, FeeConfig, Subscription, SubscriptionTier, TrialOffer};

use soroban_sdk::{contract, contractimpl, token, Address, Env, String};

const INSTANCE_TTL_MIN: u32 = 100;
const INSTANCE_TTL_MAX: u32 = 500;

#[contract]
pub struct ScoutAccessContract;

#[contractimpl]
impl ScoutAccessContract {
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
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::XlmToken, &xlm_token);
        env.storage().instance().set(&DataKey::FeeConfig, &fee_config);
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::AccumulatedFees, &0i128);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
        Ok(())
    }

    pub fn update_fee_config(env: Env, fee_config: FeeConfig) -> Result<(), ScoutAccessError> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&DataKey::FeeConfig, &fee_config);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
        Ok(())
    }

    pub fn withdraw_fees(env: Env, to: Address) -> Result<i128, ScoutAccessError> {
        Self::require_admin(&env)?;
        let fees: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedFees)
            .unwrap_or(0i128);
        if fees == 0 {
            return Ok(0);
        }
        let xlm = Self::xlm_token(&env);
        let contract_addr = env.current_contract_address();
        token::Client::new(&env, &xlm).transfer(&contract_addr, &to, &fees);
        env.storage().instance().set(&DataKey::AccumulatedFees, &0i128);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
        events::fees_withdrawn(&env, &to, fees);
        Ok(fees)
    }

    pub fn pause_contract(env: Env) -> Result<(), ScoutAccessError> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ScoutAccessError> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Scout subscription
    // -------------------------------------------------------------------------

    /// Purchase a scout subscription. Scout must pre-approve the XLM transfer.
    pub fn subscribe(
        env: Env,
        scout: Address,
        tier: SubscriptionTier,
    ) -> Result<(), ScoutAccessError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        scout.require_auth();

        let config = Self::fee_config(&env);
        let fee = match &tier {
            SubscriptionTier::Basic => config.basic_sub_stroops,
            SubscriptionTier::Pro => config.pro_sub_stroops,
            SubscriptionTier::Elite => config.elite_sub_stroops,
        };

        let xlm = Self::xlm_token(&env);
        let contract_addr = env.current_contract_address();
        token::Client::new(&env, &xlm).transfer(&scout, &contract_addr, &fee);
        Self::accumulate_fee(&env, fee);

        let now = env.ledger().timestamp();
        let sub = Subscription {
            scout: scout.clone(),
            tier: tier.clone(),
            expires_at: now
                .checked_add(config.sub_duration_secs)
                .expect("overflow"),
            subscribed_at: now,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Subscription(scout.clone()), &sub);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);

        events::scout_subscribed(&env, &scout, &tier);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Pay-to-contact
    // -------------------------------------------------------------------------

    /// Pay a micro-fee to unlock a player's contact details.
    /// Scout must have an active subscription.
    pub fn pay_to_contact(
        env: Env,
        scout: Address,
        player_id: u64,
    ) -> Result<(), ScoutAccessError> {
        Self::require_not_paused(&env)?;
        scout.require_auth();
        Self::require_active_subscription(&env, &scout)?;

        let contact_key = DataKey::ContactRecord(player_id, scout.clone());
        if env.storage().persistent().has(&contact_key) {
            return Err(ScoutAccessError::AlreadyContacted);
        }

        let config = Self::fee_config(&env);
        let xlm = Self::xlm_token(&env);
        let contract_addr = env.current_contract_address();
        token::Client::new(&env, &xlm).transfer(
            &scout,
            &contract_addr,
            &config.contact_fee_stroops,
        );
        Self::accumulate_fee(&env, config.contact_fee_stroops);

        env.storage().persistent().set(&contact_key, &true);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
        events::player_contacted(&env, player_id, &scout);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Trial offer
    // -------------------------------------------------------------------------

    /// Log a trial offer on-chain. Scout must have an Elite subscription.
    /// The backend should call progress.advance_level after this succeeds.
    pub fn log_trial_offer(
        env: Env,
        scout: Address,
        player_id: u64,
        details_hash: String,
    ) -> Result<u32, ScoutAccessError> {
        Self::require_not_paused(&env)?;
        scout.require_auth();

        let sub = Self::require_active_subscription(&env, &scout)?;
        if sub.tier != SubscriptionTier::Elite {
            return Err(ScoutAccessError::Unauthorized);
        }

        let counter_key = DataKey::TrialCounter(player_id);
        let index: u32 = env
            .storage()
            .persistent()
            .get(&counter_key)
            .unwrap_or(0u32);
        let next_index = index.checked_add(1).expect("overflow");

        let offer = TrialOffer {
            player_id,
            scout: scout.clone(),
            details_hash,
            logged_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::TrialOffer(player_id, next_index), &offer);
        env.storage().persistent().set(&counter_key, &next_index);

        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);

        events::trial_offer_logged(&env, player_id, &scout);
        Ok(next_index)
    }

    // -------------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------------

    pub fn get_subscription(
        env: Env,
        scout: Address,
    ) -> Result<Subscription, ScoutAccessError> {
        env.storage()
            .persistent()
            .get(&DataKey::Subscription(scout))
            .ok_or(ScoutAccessError::ScoutNotSubscribed)
    }

    pub fn get_fee_config(env: Env) -> FeeConfig {
        Self::fee_config(&env)
    }

    pub fn get_accumulated_fees(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::AccumulatedFees)
            .unwrap_or(0i128)
    }

    pub fn has_contacted(env: Env, scout: Address, player_id: u64) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::ContactRecord(player_id, scout))
    }

    pub fn get_trial_offer(
        env: Env,
        player_id: u64,
        index: u32,
    ) -> Result<TrialOffer, ScoutAccessError> {
        env.storage()
            .persistent()
            .get(&DataKey::TrialOffer(player_id, index))
            .ok_or(ScoutAccessError::TrialOfferNotFound)
    }

    pub fn get_trial_count(env: Env, player_id: u64) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::TrialCounter(player_id))
            .unwrap_or(0u32)
    }

    pub fn health(env: Env) -> bool {
        env.storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Initialized)
            .unwrap_or(false)
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    fn require_admin(env: &Env) -> Result<(), ScoutAccessError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(ScoutAccessError::NotInitialized)?;
        admin.require_auth();
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

    fn xlm_token(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::XlmToken)
            .expect("xlm token not set")
    }

    fn accumulate_fee(env: &Env, amount: i128) {
        let current: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedFees)
            .unwrap_or(0i128);
        env.storage()
            .instance()
            .set(&DataKey::AccumulatedFees, &(current + amount));
    }
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
        token::{Client as TokenClient, StellarAssetClient},
        Env, IntoVal, String,
    };

    /// Deploy a mock SAC token, mint `amount` to `to`, return the token contract address.
    fn create_token(env: &Env, admin: &Address) -> Address {
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        token_id.address()
    }

    fn mint_token(env: &Env, token: &Address, admin: &Address, to: &Address, amount: i128) {
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

    fn setup() -> (Env, Address, Address, Address, ScoutAccessContractClient<'static>) {
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
    fn test_initialize_and_health() {
        let (_, _, _, _, client) = setup();
        assert!(client.health());
    }

    #[test]
    fn test_subscribe_basic() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        // Fund scout with enough XLM
        mint_token(&env, &xlm, &admin, &scout, 10_000_000);

        client.subscribe(&scout, &SubscriptionTier::Basic);

        let sub = client.get_subscription(&scout);
        assert_eq!(sub.tier, SubscriptionTier::Basic);
        assert!(sub.expires_at > sub.subscribed_at);
        assert_eq!(client.get_accumulated_fees(), 1_000_000);
    }

    #[test]
    fn test_subscribe_elite_and_pay_to_contact() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);

        assert!(client.has_contacted(&scout, &1u64));
        // elite fee + contact fee
        assert_eq!(client.get_accumulated_fees(), 7_000_000 + 100_000);
    }

    #[test]
    #[should_panic]
    fn test_duplicate_contact_fails() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &1u64);
        // second contact with same player should panic
        client.pay_to_contact(&scout, &1u64);
    }

    #[test]
    fn test_log_trial_offer_elite() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        client.subscribe(&scout, &SubscriptionTier::Elite);
        let idx = client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmTrialDetails"),
        );
        assert_eq!(idx, 1);
        assert_eq!(client.get_trial_count(&1u64), 1);

        let offer = client.get_trial_offer(&1u64, &1u32);
        assert_eq!(offer.player_id, 1);
        assert_eq!(offer.scout, scout);
    }

    #[test]
    #[should_panic]
    fn test_trial_offer_requires_elite() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        // Pro tier — not allowed to log trial offers
        client.subscribe(&scout, &SubscriptionTier::Pro);
        client.log_trial_offer(&scout, &1u64, &String::from_str(&env, "QmDetails"));
    }

    #[test]
    #[should_panic]
    fn test_contact_without_subscription_fails() {
        let (env, _, _, _, client) = setup();
        let scout = Address::generate(&env);
        client.pay_to_contact(&scout, &1u64);
    }

    #[test]
    fn test_subscription_expiry() {
        let (env, admin, xlm, contract_id, client) = setup();
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
    fn test_instance_ttl_extended_after_ledger_advancement() {
        let (env, admin, xlm, contract_id, client) = setup();
        let scout = Address::generate(&env);
        mint_token(&env, &xlm, &admin, &scout, 100_000_000);

        // Verify contract is initialized
        assert!(client.health());

        // Advance ledger by 600 blocks (more than INSTANCE_TTL_MAX)
        env.ledger().with_mut(|l| {
            l.sequence += 600;
        });

        // Contract should still be functional because TTL was extended
        client.subscribe(&scout, &SubscriptionTier::Elite);
        assert!(client.health());

        // Verify pause/unpause still works
        client.pause_contract();
        assert!(client.health());
        client.unpause_contract();
        assert!(client.health());

        // Verify trial offer logging still works
        let idx = client.log_trial_offer(
            &scout,
            &1u64,
            &String::from_str(&env, "QmTrialDetails"),
        );
        assert_eq!(idx, 1);
    }
}

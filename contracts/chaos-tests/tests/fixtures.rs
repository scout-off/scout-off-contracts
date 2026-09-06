use std::collections::BTreeMap;

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, String, Vec};

/// Contact / subscription fees used by the harness (stroops).
pub const CONTACT_FEE: i128 = 100_000;
pub const BASIC_SUB: i128 = 1_000_000;
pub const PRO_SUB: i128 = 3_000_000;
pub const ELITE_SUB: i128 = 7_000_000;
pub const TRIAL_ESCROW: i128 = 500_000;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: CONTACT_FEE,
        basic_sub_stroops: BASIC_SUB,
        pro_sub_stroops: PRO_SUB,
        elite_sub_stroops: ELITE_SUB,
        sub_duration_secs: 30 * 24 * 60 * 60,
        pro_contact_limit: 50,
        trial_offer_escrow_stroops: TRIAL_ESCROW,
        trial_offer_expiry_secs: 7_200,
    }
}

/// Shared entity pool + running totals the invariant checkers consult.
pub struct Harness {
    pub env: Env,
    pub xlm: Address,
    pub players: Vec<Address>,
    pub player_ids: std::vec::Vec<u64>,
    pub scouts: Vec<Address>,
    pub validators: Vec<Address>,
    pub progress: ProgressContractClient<'static>,
    pub registration: RegistrationContractClient<'static>,
    pub scout_access: ScoutAccessContractClient<'static>,
    pub verification: VerificationContractClient<'static>,
    /// Independently tracked sum of fee-generating events minus
    /// withdrawals / refunds executed by this harness.
    pub expected_fees: i128,
    /// Accumulated-fees value last observed after a successful fee-affecting op.
    pub last_observed_fees: i128,
    /// Sum of withdraw_fees / refund_subscription amounts applied this schedule.
    pub fees_withdrawn_or_refunded: i128,
    /// True if get_accumulated_fees() ever dropped without a matching withdraw/refund.
    pub fee_counter_regressed: bool,
    /// Per-player level last observed by the harness (between invariant checks).
    pub last_observed_levels: BTreeMap<u64, ProgressLevel>,
    /// Monotonic seed used to mint unique valid CIDv0 strings.
    cid_seed: u32,
}

impl Harness {
    pub fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);

        let progress_id = env.register(ProgressContract, ());
        let progress = ProgressContractClient::new(&env, &progress_id);
        progress.initialize(&admin);

        let reg_id = env.register(RegistrationContract, ());
        let registration = RegistrationContractClient::new(&env, &reg_id);
        registration.initialize(&admin);

        let ver_id = env.register(VerificationContract, ());
        let verification = VerificationContractClient::new(&env, &ver_id);
        verification.initialize(&admin);

        let xlm = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let sa_id = env.register(ScoutAccessContract, ());
        let scout_access = ScoutAccessContractClient::new(&env, &sa_id);
        scout_access.initialize(&admin, &xlm, &default_fees());

        // Wire the four contracts so milestone approval can actually advance
        // levels and so fee collection is a real token transfer.
        verification.set_progress_contract(&progress_id);
        progress.set_verification_contract(&ver_id);
        progress.set_registration_contract(&reg_id);
        progress.set_scout_access_contract(&sa_id);
        scout_access.set_progress_contract(&progress_id);
        registration.set_progress_contract(&progress_id);

        let mut players = Vec::new(&env);
        let mut player_ids = std::vec::Vec::new();
        for _ in 0..3 {
            let wallet = Address::generate(&env);
            let pid = registration.register_player(
                &wallet,
                &PlayerVitals {
                    age: 20,
                    position: String::from_str(&env, "Forward"),
                    region: String::from_str(&env, "West Africa"),
                    nationality: String::from_str(&env, "Ghana"),
                },
                &{
                    let mut hashes = Vec::new(&env);
                    hashes.push_back(String::from_str(&env, "QmCID1"));
                    hashes
                },
            );
            players.push_back(wallet);
            player_ids.push(pid);
        }

        let mut scouts = Vec::new(&env);
        for _ in 0..2 {
            let wallet = Address::generate(&env);
            let _ = registration.register_scout(&wallet, &String::from_str(&env, "West Africa"));
            StellarAssetClient::new(&env, &xlm).mint(&wallet, &100_000_000i128);
            scout_access.subscribe(&wallet, &SubscriptionTier::Elite);
            scouts.push_back(wallet);
        }

        let mut validators = Vec::new(&env);
        for _ in 0..2 {
            let wallet = Address::generate(&env);
            verification.register_validator(
                &wallet,
                &String::from_str(&env, "UEFA B License"),
                &String::from_str(&env, "Default Academy"),
                &Vec::new(&env),
            );
            validators.push_back(wallet);
        }

        let expected_fees = ELITE_SUB * 2;
        let last_observed_fees = scout_access.get_accumulated_fees();

        let mut last_observed_levels = BTreeMap::new();
        for &pid in &player_ids {
            last_observed_levels.insert(pid, progress.get_level(&pid));
        }

        Self {
            env,
            xlm,
            players,
            player_ids,
            scouts,
            validators,
            progress,
            registration,
            scout_access,
            verification,
            expected_fees,
            last_observed_fees,
            fees_withdrawn_or_refunded: 0,
            fee_counter_regressed: false,
            last_observed_levels,
            cid_seed: 0,
        }
    }

    /// Record a successful fee-generating event (`delta > 0`) or a
    /// withdrawal/refund (`delta < 0`). Updates the expected total and
    /// checks that the on-chain counter moved in the same direction.
    pub fn record_fee_delta(&mut self, delta: i128) {
        self.expected_fees = self.expected_fees.saturating_add(delta);
        if delta < 0 {
            self.fees_withdrawn_or_refunded =
                self.fees_withdrawn_or_refunded.saturating_add(delta.abs());
        }
        let actual = self.scout_access.get_accumulated_fees();
        if actual < self.last_observed_fees && delta >= 0 {
            self.fee_counter_regressed = true;
        }
        self.last_observed_fees = actual;
    }

    /// Snapshot every known player's current level into `last_observed_levels`.
    pub fn snapshot_levels(&mut self) {
        self.last_observed_levels.clear();
        for &pid in &self.player_ids {
            self.last_observed_levels
                .insert(pid, self.progress.get_level(&pid));
        }
    }

    /// Deterministically generate a syntactically valid CIDv0 (46 chars,
    /// "Qm" prefix, base58btc charset) so successive approve/log calls
    /// don't collide on the global evidence-hash uniqueness constraint.
    pub fn next_cid(&mut self) -> String {
        const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let mut buf = [0u8; 46];
        buf[0] = b'Q';
        buf[1] = b'm';
        self.cid_seed = self.cid_seed.wrapping_add(1);
        let mut x = self.cid_seed;
        for slot in buf.iter_mut().skip(2) {
            *slot = ALPHABET[(x as usize) % ALPHABET.len()];
            x = x.wrapping_mul(31).wrapping_add(7);
        }
        String::from_str(&self.env, core::str::from_utf8(&buf).unwrap())
    }
}

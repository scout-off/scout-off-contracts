use scoutchain_scout_access::{FeeConfig, ScoutAccessContract, ScoutAccessError};
use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env};
use std::panic::{catch_unwind, AssertUnwindSafe};

#[contract]
struct MockToken;

#[contractimpl]
impl MockToken {
    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&0u32).unwrap_or(7)
    }
    pub fn set_decimals(env: Env, d: u32) {
        env.storage().instance().set(&0u32, &d);
    }
}

fn fees() -> FeeConfig {
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

fn try_init(decimals: u32) -> Option<String> {
    let env = Env::default();
    let token = env.register(MockToken, ());
    MockTokenClient::new(&env, &token).set_decimals(&decimals);
    let args = (Address::generate(&env), token, fees());
    catch_unwind(AssertUnwindSafe(|| env.register(ScoutAccessContract, args)))
        .err()
        .map(|e| format!("{e:?}"))
}

#[test]
fn seven_decimals_accepted() {
    assert!(try_init(7).is_none());
}

#[test]
fn two_and_eighteen_decimals_rejected() {
    // Constructor failures surface as a panic carrying the contract error code.
    let code = ScoutAccessError::UnsupportedTokenDecimals as u32;
    assert!(try_init(2).is_some(), "expected #{code}");
    assert!(try_init(18).is_some(), "expected #{code}");
}

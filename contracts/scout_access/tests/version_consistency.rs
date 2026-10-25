use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{RegistrationContract, RegistrationContractClient};
use scoutchain_scout_access::{FeeConfig, ScoutAccessContract, ScoutAccessContractClient};
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

#[test]
fn test_all_contracts_version_consistency() {
    let env = Env::default();

    // Deploy all four contracts
    let admin = Address::generate(&env);
    let xlm = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let reg_id = env.register(RegistrationContract, (admin.clone(),));
    let ver_id = env.register(VerificationContract, (admin.clone(),));
    let prog_id = env.register(ProgressContract, (admin.clone(),));
    let sa_id = env.register(
        ScoutAccessContract,
        (
            admin,
            xlm,
            FeeConfig {
                contact_fee_stroops: 100_000,
                basic_sub_stroops: 1_000_000,
                pro_sub_stroops: 3_000_000,
                elite_sub_stroops: 7_000_000,
                sub_duration_secs: 30 * 24 * 60 * 60,
                pro_contact_limit: 10,
                trial_offer_escrow_stroops: 500_000,
                trial_offer_expiry_secs: 3_600,
            },
        ),
    );

    let reg_client = RegistrationContractClient::new(&env, &reg_id);
    let ver_client = VerificationContractClient::new(&env, &ver_id);
    let prog_client = ProgressContractClient::new(&env, &prog_id);
    let sa_client = ScoutAccessContractClient::new(&env, &sa_id);

    // Call version() on each contract
    let reg_ver = reg_client.version();
    let ver_ver = ver_client.version();
    let prog_ver = prog_client.version();
    let sa_ver = sa_client.version();

    // Expected workspace version from CARGO_PKG_VERSION at compile time
    let expected_ver = String::from_str(&env, env!("CARGO_PKG_VERSION"));

    // Assert all four contracts report the exact same version string
    assert_eq!(
        reg_ver, ver_ver,
        "Registration and Verification contract versions differ"
    );
    assert_eq!(
        ver_ver, prog_ver,
        "Verification and Progress contract versions differ"
    );
    assert_eq!(
        prog_ver, sa_ver,
        "Progress and ScoutAccess contract versions differ"
    );

    // Assert each contract matches the workspace Cargo.toml version
    assert_eq!(
        reg_ver, expected_ver,
        "Contract version does not match workspace CARGO_PKG_VERSION"
    );

    // Assert non-empty
    assert!(!reg_ver.is_empty(), "Contract version string is empty");
}

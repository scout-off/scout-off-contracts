use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{RegistrationContract, RegistrationContractClient};
use scoutchain_scout_access::{ScoutAccessContract, ScoutAccessContractClient};
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{Env, String};

#[test]
fn test_all_contracts_version_consistency() {
    let env = Env::default();

    // Deploy all four contracts
    let reg_id = env.register(RegistrationContract, ());
    let ver_id = env.register(VerificationContract, ());
    let prog_id = env.register(ProgressContract, ());
    let sa_id = env.register(ScoutAccessContract, ());

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

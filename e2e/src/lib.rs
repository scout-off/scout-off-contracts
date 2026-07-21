use soroban_sdk::{
    contractimport, testutils::Address as _, testutils::Events, testutils::Ledger, Address, Env,
    String,
};

use scoutchain_progress::ProgressContract;
use scoutchain_registration::{types::PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_scout_access::{FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier};
use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{VerificationContract, VerificationContractClient};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_player_lifecycle_register_verify_and_progress() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 1);

        let admin = Address::generate(&env);
        let validator = Address::generate(&env);
        let player = Address::generate(&env);
        let scout = Address::generate(&env);

        let reg_id = env.register_contract(None, RegistrationContract);
        let ver_id = env.register_contract(None, VerificationContract);
        let progress_id = env.register_contract(None, ProgressContract);
        let sa_id = env.register_contract(None, ScoutAccessContract);

        let reg = RegistrationContractClient::new(&env, &reg_id);
        let ver = VerificationContractClient::new(&env, &ver_id);
        let progress = scoutchain_progress::ProgressContractClient::new(&env, &progress_id);
        let sa = ScoutAccessContractClient::new(&env, &sa_id);

        reg.initialize(&admin).unwrap();
        ver.initialize(&admin).unwrap();
        progress.initialize(&admin).unwrap();
        sa.initialize(
            &admin,
            &Address::generate(&env),
            &FeeConfig {
                contact_fee_stroops: 1_000_000,
                subscription_fee_stroops: 1_000_000,
                subscription_tier: SubscriptionTier::Pro,
            },
        )
        .unwrap();

        reg.set_progress_contract(&progress_id).unwrap();
        ver.set_progress_contract(&progress_id).unwrap();
        progress.set_verification_contract(&ver_id).unwrap();
        progress.set_registration_contract(&reg_id).unwrap();
        progress.set_scout_access_contract(&sa_id).unwrap();

        ver.register_validator(&validator, &String::from_str(&env, "UEFA B License")).unwrap();

        let vitals = PlayerVitals {
            name: String::from_str(&env, "Ari"),
            position: String::from_str(&env, "ST"),
            region: String::from_str(&env, "NA"),
            nationality: String::from_str(&env, "US"),
            age: 20,
            hashes: Vec::new(&env),
        };

        let player_id = reg.register_player(&player, &vitals, &Vec::from_array(&env, [String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB")])).unwrap();

        let milestone_id = ver.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "scored"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
        assert_eq!(milestone_id, 1);

        let level = progress.get_level(&player_id);
        assert_eq!(level, ProgressLevel::VerifiedIdentity);

        let _ = sa.contact_player(&scout, &player_id).unwrap();
        let contacts = sa.get_scout_contacts(&scout);
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts.get(0).unwrap(), player_id);
    }
}

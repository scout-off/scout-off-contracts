import re

with open("contracts/verification/src/lib.rs", "r") as f:
    content = f.read()

# Add get_diversity_config and set_diversity_config
diversity_code = """
    pub fn get_diversity_config(env: Env) -> Option<DiversityConfig> {
        env.storage()
            .persistent()
            .get(&DataKey::DiversityConfig)
    }

    pub fn set_diversity_config(
        env: Env,
        required_distinct_affiliations: u32,
        starting_milestone_index: u32,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let config = DiversityConfig {
            required_distinct_affiliations,
            starting_milestone_index,
        };
        env.storage()
            .persistent()
            .set(&DataKey::DiversityConfig, &config);
        Ok(())
    }
"""

content = content.replace("pub fn set_progress_contract(", diversity_code + "\n    pub fn set_progress_contract(")

# Fix batch_register_validators signature
content = content.replace(
    """pub fn batch_register_validators(
        env: Env,
        entries: Vec<(Address, String, Vec<String>)>,
    ) -> Result<(), VerificationError> {""",
    """pub fn batch_register_validators(
        env: Env,
        entries: Vec<(Address, String, String, Vec<String>)>,
    ) -> Result<(), VerificationError> {"""
)

# Fix loop in batch_register_validators
content = content.replace(
    "let (wallet, credentials, specializations) = entries.get(i).unwrap();",
    "let (wallet, credentials, affiliation, specializations) = entries.get(i).unwrap();\n            if affiliation.len() > MAX_CREDENTIALS_LEN {\n                return Err(VerificationError::InvalidInput);\n            }"
)
content = content.replace(
    "let (other_wallet, _, _) = entries.get(j).unwrap();",
    "let (other_wallet, _, _, _) = entries.get(j).unwrap();"
)

# Fix Validator struct initialization in batch_register_validators
content = content.replace(
    """let validator = Validator {
                wallet: wallet.clone(),
                credentials: credentials.clone(),
                registered_at: env.ledger().timestamp(),""",
    """let validator = Validator {
                wallet: wallet.clone(),
                credentials: credentials.clone(),
                affiliation: affiliation.clone(),
                registered_at: env.ledger().timestamp(),"""
)

# Fix admin_transfer_properties (Validator initialization)
content = content.replace(
    """let new_validator = Validator {
            wallet: new_wallet.clone(),
            credentials: old_validator.credentials.clone(),
            registered_at: old_validator.registered_at,""",
    """let new_validator = Validator {
            wallet: new_wallet.clone(),
            credentials: old_validator.credentials.clone(),
            affiliation: old_validator.affiliation.clone(),
            registered_at: old_validator.registered_at,"""
)

# Fix register_validator signature
content = content.replace(
    """pub fn register_validator(
        env: Env,
        wallet: Address,
        credentials: String,
        specializations: Vec<String>,
    ) -> Result<(), VerificationError> {""",
    """pub fn register_validator(
        env: Env,
        wallet: Address,
        credentials: String,
        affiliation: String,
        specializations: Vec<String>,
    ) -> Result<(), VerificationError> {"""
)
content = content.replace(
    """if credentials.len() < MIN_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }""",
    """if credentials.len() < MIN_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }
        
        if affiliation.len() > MAX_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }"""
)
content = content.replace(
    """let validator = Validator {
            wallet: wallet.clone(),
            credentials,
            registered_at: env.ledger().timestamp(),""",
    """let validator = Validator {
            wallet: wallet.clone(),
            credentials,
            affiliation,
            registered_at: env.ledger().timestamp(),"""
)

# Fix commit_approved_milestone
commit_logic = """
        let validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(validator_wallet.clone()))
            .unwrap();

        let mut player_affiliations: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerAffiliations(player_id))
            .unwrap_or_else(|| Vec::new(env));

        if !player_affiliations.contains(&validator.affiliation) {
            player_affiliations.push_back(validator.affiliation.clone());
            env.storage()
                .persistent()
                .set(&DataKey::PlayerAffiliations(player_id), &player_affiliations);
        }

        let diversity_config = Self::get_diversity_config(env.clone());
        let mut advance_allowed = true;
        if let Some(config) = diversity_config {
            if next_index >= config.starting_milestone_index {
                if player_affiliations.len() < config.required_distinct_affiliations {
                    advance_allowed = false;
                }
            }
        }

        if advance_allowed {
            if let Some(progress_addr) = env
                .storage()
                .instance()
                .get::<DataKey, Address>(&DataKey::ProgressContract)
            {
                let progress_client = progress_contract::Client::new(env, &progress_addr);
                match progress_client.try_advance_level(validator_wallet, &player_id, &next_index) {
                    Ok(_) => {}
                    Err(Ok(progress_contract::ProgressError::AlreadyAtMaxLevel)) => {
                        events::level_advancement_skipped(
                            env,
                            player_id,
                            &soroban_sdk::String::from_str(env, "AlreadyAtMaxLevel"),
                        );
                    }
                    Err(e) => {
                        let code = match &e {
                            Ok(pe) => *pe as u32,
                            Err(_) => 0u32,
                        };
                        events::progress_call_failed(env, player_id, code);
                        return Err(VerificationError::ProgressCallFailed);
                    }
                }
            } else {
                if !env.storage().instance().has(&DataKey::ProgressContract) {
                    events::progress_contract_not_set(env, player_id);
                }
            }
        } else {
            if !env.storage().instance().has(&DataKey::ProgressContract) {
                events::progress_contract_not_set(env, player_id);
            }
        }
"""

old_commit_logic = """
        if let Some(progress_addr) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::ProgressContract)
        {
            let progress_client = progress_contract::Client::new(env, &progress_addr);
            match progress_client.try_advance_level(validator_wallet, &player_id, &next_index) {
                Ok(_) => {}
                Err(Ok(progress_contract::ProgressError::AlreadyAtMaxLevel)) => {
                    events::level_advancement_skipped(
                        env,
                        player_id,
                        &soroban_sdk::String::from_str(env, "AlreadyAtMaxLevel"),
                    );
                }
                Err(e) => {
                    let code = match &e {
                        Ok(pe) => *pe as u32,
                        Err(_) => 0u32,
                    };
                    events::progress_call_failed(env, player_id, code);
                    return Err(VerificationError::ProgressCallFailed);
                }
            }
        } else {
            events::progress_contract_not_set(env, player_id);
        }
"""
content = content.replace(old_commit_logic.strip(), commit_logic.strip())

with open("contracts/verification/src/lib.rs", "w") as f:
    f.write(content)

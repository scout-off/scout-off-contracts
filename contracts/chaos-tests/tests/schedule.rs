use scoutchain_registration::PlayerVitals;
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, String, Vec};

use crate::fixtures::{Harness, CONTACT_FEE};

#[derive(Debug, Clone, Copy)]
pub enum Operation {
    ApproveMilestone {
        player_idx: usize,
        validator_idx: usize,
    },
    RegisterPlayer,
    RegisterScout,
    ContactPlayer {
        scout_idx: usize,
        player_idx: usize,
    },
    LogTrialOffer {
        scout_idx: usize,
        player_idx: usize,
    },
}

pub struct ScheduleGenerator {
    seed: u64,
}

impl ScheduleGenerator {
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub fn generate(&mut self, max_ops: u32) -> std::vec::Vec<Operation> {
        let mut ops = std::vec::Vec::new();
        for _ in 0..max_ops {
            self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let op_type = (self.seed % 5) as usize;
            match op_type {
                0 => ops.push(Operation::ApproveMilestone {
                    player_idx: (self.seed % 3) as usize,
                    validator_idx: (self.seed % 2) as usize,
                }),
                1 => ops.push(Operation::RegisterPlayer),
                2 => ops.push(Operation::RegisterScout),
                3 => ops.push(Operation::ContactPlayer {
                    scout_idx: (self.seed % 2) as usize,
                    player_idx: (self.seed % 3) as usize,
                }),
                4 => ops.push(Operation::LogTrialOffer {
                    scout_idx: (self.seed % 2) as usize,
                    player_idx: (self.seed % 3) as usize,
                }),
                _ => unreachable!(),
            }
        }
        ops
    }
}

impl Harness {
    pub fn apply(&mut self, op: &Operation) -> Result<(), std::string::String> {
        match op {
            Operation::ApproveMilestone {
                player_idx,
                validator_idx,
            } => {
                let player_id = *self
                    .player_ids
                    .get(*player_idx)
                    .ok_or_else(|| format!("player_idx {player_idx} out of range"))?;
                let validator = self.validators.get(*validator_idx as u32).unwrap();
                let cid = self.next_cid();
                let result = self.verification.try_approve_milestone(
                    &validator,
                    &player_id,
                    &String::from_str(&self.env, "chaos-test"),
                    &cid,
                    &None,
                );
                match result {
                    // Prior level stays in last_observed_levels so the
                    // end-of-schedule monotonicity check can see the delta.
                    Ok(Ok(_)) => Ok(()),
                    other => Err(format!("approve_milestone failed: {:?}", other)),
                }
            }
            Operation::ContactPlayer {
                scout_idx,
                player_idx,
            } => {
                let scout = self.scouts.get(*scout_idx as u32).unwrap();
                let player_id = *self
                    .player_ids
                    .get(*player_idx)
                    .ok_or_else(|| format!("player_idx {player_idx} out of range"))?;
                StellarAssetClient::new(&self.env, &self.xlm).mint(&scout, &CONTACT_FEE);
                let result = self.scout_access.try_pay_to_contact(&scout, &player_id);
                match result {
                    Ok(Ok(())) => {
                        self.record_fee_delta(CONTACT_FEE);
                        Ok(())
                    }
                    other => Err(format!("pay_to_contact failed: {:?}", other)),
                }
            }
            Operation::LogTrialOffer {
                scout_idx,
                player_idx,
            } => {
                let scout = self.scouts.get(*scout_idx as u32).unwrap();
                let player_id = *self
                    .player_ids
                    .get(*player_idx)
                    .ok_or_else(|| format!("player_idx {player_idx} out of range"))?;
                let cid = self.next_cid();
                let result = self
                    .scout_access
                    .try_log_trial_offer(&scout, &player_id, &cid);
                match result {
                    Ok(Ok(_)) => Ok(()),
                    other => Err(format!("log_trial_offer failed: {:?}", other)),
                }
            }
            Operation::RegisterPlayer => {
                let wallet = Address::generate(&self.env);
                let mut hashes = Vec::new(&self.env);
                hashes.push_back(String::from_str(&self.env, "QmCID2"));
                let result = self.registration.try_register_player(
                    &wallet,
                    &PlayerVitals {
                        age: 20,
                        position: String::from_str(&self.env, "Midfielder"),
                        region: String::from_str(&self.env, "East Africa"),
                        nationality: String::from_str(&self.env, "Kenya"),
                    },
                    &hashes,
                );
                if let Ok(Ok(pid)) = result {
                    self.players.push_back(wallet);
                    self.player_ids.push(pid);
                    self.last_observed_levels
                        .insert(pid, ProgressLevel::Unverified);
                }
                Ok(())
            }
            Operation::RegisterScout => {
                let wallet = Address::generate(&self.env);
                let result = self
                    .registration
                    .try_register_scout(&wallet, &String::from_str(&self.env, "North Africa"));
                if result.is_ok() {
                    self.scouts.push_back(wallet);
                }
                Ok(())
            }
        }
    }
}

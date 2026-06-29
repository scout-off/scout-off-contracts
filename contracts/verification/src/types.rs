pub use scoutchain_shared_types::ContractHealth;
use soroban_sdk::{contracttype, Address, String};

/// Richer validator status — distinguishes unregistered from revoked.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ValidatorStatus {
    NotRegistered,
    Active,
    Revoked,
}

/// A single verified milestone record
#[contracttype]
#[derive(Clone, Debug)]
pub struct Milestone {
    pub player_id: u64,
    pub validator: Address,
    pub description: String,
    /// IPFS/Arweave CID of supporting evidence (video clip, stat sheet, etc.)
    pub evidence_hash: String,
    pub approved_at: u64,
    /// Stellar ledger sequence at time of approval for tamper-proof auditability
    pub ledger_sequence: u32,
}

/// Validator entry in the trusted registry
#[contracttype]
#[derive(Clone, Debug)]
pub struct Validator {
    pub wallet: Address,
    /// Human-readable credential label (e.g. "UEFA B License", "Academy Director")
    pub credentials: String,
    pub registered_at: u64,
    pub active: bool,
}

/// Dispute record for a milestone (issue #471)
#[contracttype]
#[derive(Clone, Debug)]
pub struct MilestoneDispute {
    pub player_id: u64,
    pub milestone_index: u32,
    pub reason: String,
    pub disputed_at: u64,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Initialized,
    Paused,
    ProgressContract,
    ProgressContractSet,
    Validator(Address),
    MilestoneCounter(u64),
    Milestone(u64, u32),
    ValidatorMilestoneCount(Address),
    ValidatorPlayerMilestoneCount(Address, u64),
    /// Milestone dispute record: (player_id, milestone_index) → MilestoneDispute
    MilestoneDispute(u64, u32),
    ValidatorVector,
    TotalMilestoneCount,
}

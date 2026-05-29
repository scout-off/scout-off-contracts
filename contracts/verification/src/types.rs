use soroban_sdk::{contracttype, Address, String};

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

#[contracttype]
pub enum DataKey {
    Admin,
    Initialized,
    Paused,
    /// validator wallet → Validator
    Validator(Address),
    /// milestone counter per player
    MilestoneCounter(u64),
    /// (player_id, milestone_index) → Milestone
    Milestone(u64, u32),
    /// registration contract address (cross-contract calls)
    RegistrationContract,
    /// milestone count per validator wallet
    ValidatorMilestoneCount(Address),
}

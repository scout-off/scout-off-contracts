use soroban_sdk::{contracttype, Address, String, Vec};

/// Four-tier progress level for a player profile
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ProgressLevel {
    /// Level 0 — profile created, no verification yet
    Unverified,
    /// Level 1 — identity confirmed by academy or KYC
    VerifiedIdentity,
    /// Level 2 — performance milestones verified by approved third party
    PerformanceMilestones,
    /// Level 3 — scout feedback or trial offer logged
    EliteTier,
}

/// Basic player vitals stored on-chain
#[contracttype]
#[derive(Clone, Debug)]
pub struct PlayerVitals {
    pub age: u32,
    pub position: String,
    pub region: String,
    pub nationality: String,
    /// Height in centimetres. If provided must be in [100, 250].
    pub height_cm: Option<u32>,
    /// Weight in kilograms. If provided must be in [30, 200].
    pub weight_kg: Option<u32>,
}

/// Full on-chain player profile
#[contracttype]
#[derive(Clone, Debug)]
pub struct PlayerProfile {
    pub player_id: u64,
    pub wallet: Address,
    pub vitals: PlayerVitals,
    /// IPFS/Arweave CIDs for highlight reels and photos
    pub ipfs_hashes: Vec<String>,
    pub level: ProgressLevel,
    pub registered_at: u64,
    pub updated_at: u64,
}

/// Scout profile stored on-chain
#[contracttype]
#[derive(Clone, Debug)]
pub struct ScoutProfile {
    pub scout_id: u64,
    pub wallet: Address,
    pub region: String,
    pub registered_at: u64,
}

/// Storage keys
#[contracttype]
pub enum DataKey {
    Admin,
    Initialized,
    Paused,
    PlayerCounter,
    ScoutCounter,
    Player(u64),
    /// Index: wallet → player_id
    PlayerByWallet(Address),
    Scout(u64),
    ScoutByWallet(Address),
}

use soroban_sdk::{contracttype, Address, String, Vec};

pub use scoutchain_shared_types::ProgressLevel;

/// Basic player vitals stored on-chain
#[contracttype]
#[derive(Clone, Debug)]
pub struct PlayerVitals {
    pub age: u32,
    pub position: String,
    pub region: String,
    pub nationality: String,
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
    pub organization: String,
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

use soroban_sdk::{contracttype, Address, String, Vec};

pub use scoutchain_shared_types::{ContractHealth, ProgressLevel};

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
    pub verified: bool,
    pub registered_at: u64,
}

/// Storage keys for contract state
#[contracttype]
pub enum DataKey {
    /// Admin wallet address authorized to manage validators and fees
    Admin,
    /// Boolean flag indicating if contract has been initialized
    Initialized,
    /// Boolean flag indicating if contract is paused (circuit breaker)
    Paused,
    /// Counter for generating unique player IDs
    PlayerCounter,
    /// Counter for generating unique scout IDs
    ScoutCounter,
    /// Full player profile stored by player_id
    Player(u64),
    /// Index mapping player wallet address to player_id for fast lookup
    PlayerByWallet(Address),
    /// Full scout profile stored by scout_id
    Scout(u64),
    /// Index mapping scout wallet address to scout_id for fast lookup
    ScoutByWallet(Address),
    /// Index of all player IDs for efficient filtering and iteration
    PlayerIndex,
    /// Address of the progress contract allowed to call set_player_level
    ProgressContract,
}

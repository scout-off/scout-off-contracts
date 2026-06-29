use soroban_sdk::{contracttype, Address, String};

/// Subscription tier for scouts
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum SubscriptionTier {
    /// Basic — browse verified players (Level 1+)
    Basic,
    /// Pro — browse all levels + contact up to 10 players/month
    Pro,
    /// Elite — unlimited contacts + trial offer logging
    Elite,
}

/// Active scout subscription record
#[contracttype]
#[derive(Clone, Debug)]
pub struct Subscription {
    pub scout: Address,
    pub tier: SubscriptionTier,
    pub expires_at: u64,
    pub subscribed_at: u64,
}

/// A logged trial offer from a scout to a player
#[contracttype]
#[derive(Clone, Debug)]
pub struct TrialOffer {
    pub player_id: u64,
    pub scout: Address,
    /// IPFS/Arweave CID of the offer details document
    pub details_hash: String,
    pub logged_at: u64,
}

/// Platform fee configuration
#[contracttype]
#[derive(Clone, Debug)]
pub struct FeeConfig {
    /// Contact fee in stroops (1 XLM = 10_000_000 stroops)
    pub contact_fee_stroops: i128,
    /// Basic subscription fee in stroops
    pub basic_sub_stroops: i128,
    /// Pro subscription fee in stroops
    pub pro_sub_stroops: i128,
    /// Elite subscription fee in stroops
    pub elite_sub_stroops: i128,
    /// Subscription duration in seconds (default: 30 days)
    pub sub_duration_secs: u64,
    /// Maximum contacts per month for Pro tier (default: 10)
    pub pro_contact_limit: u32,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Initialized,
    Paused,
    FeeConfig,
    AccumulatedFees,
    /// Native XLM token contract address
    XlmToken,
    /// scout wallet → Subscription
    Subscription(Address),
    /// (player_id, scout) → bool (has contacted)
    ContactRecord(u64, Address),
    /// scout → Vec<u64> of contacted player_ids
    ScoutContacts(Address),
    /// Monthly contact count for Pro tier: (scout, month_bucket) → count
    ContactCount(Address, u64),
    /// trial offer counter per player
    TrialCounter(u64),
    /// (player_id, trial_index) → TrialOffer
    TrialOffer(u64, u32),
    /// progress contract address for cross-contract advance_level call
    ProgressContract,
    /// scout → Vec<u64> of contacted player_ids
    ScoutContacts(Address),
    /// (scout, player_id) → u64 timestamp of the last trial offer sent
    /// Used to enforce the per-(scout, player) cooldown window.
    TrialOfferLastSent(Address, u64),
}

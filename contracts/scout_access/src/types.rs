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

/// A recorded contact event from a scout to a player
#[contracttype]
#[derive(Clone, Debug)]
pub struct ContactRecord {
    pub player_id: u64,
    pub scout: Address,
    /// Ledger timestamp at the moment the contact was recorded
    pub contacted_at: u64,
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

/// Tracks the number of contacts a Pro-tier scout has made in their current
/// subscription period.  `period_start` is the `subscribed_at` timestamp of
/// the current subscription; when the scout renews, a new record is stored
/// (keyed by the new `subscribed_at`), effectively resetting the counter.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProContactPeriod {
    /// `subscribed_at` of the subscription this counter belongs to.
    /// Used to detect period rollovers on subscription renewal.
    pub period_start: u64,
    /// Number of contacts made in this period.
    pub count: u32,
}

/// Paginated response for scout-contact lookups.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PaginatedPlayerIds {
    pub items: soroban_sdk::Vec<u64>,
    pub next_cursor: u64,
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
    /// (scout, player_id) → u64 timestamp of the last trial offer sent
    /// Used to enforce the per-(scout, player) cooldown window.
    TrialOfferLastSent(Address, u64),
    /// tier → Vec<Address> of scouts subscribed at this tier
    TierSubscribers(SubscriptionTier),
    /// Pro-tier contact period counter: scout → ProContactPeriod
    ProContactCount(Address),
    /// player_id → Vec<Address> of scouts who have contacted this player
    PlayerContacts(u64),
    /// scout → Vec<(player_id, trial_index)> of all trial offers sent
    ScoutTrialOffers(Address),
}

use soroban_sdk::{contracttype, Address, String};

pub use scoutchain_shared_types::WiringLink;

/// Paginated response for `get_scout_contacts_page`.
///
/// Follows the `{ entries, total }` convention established by
/// [`verification::GlobalMilestoneIndexPage`] so callers have a consistent
/// stop condition when walking pages.
///
/// `entries` contains at most 50 player IDs per page; `total` is the full
/// number of players the scout has ever contacted.
///
/// NOTE: this is intentionally kept as a contract-local struct rather than
/// migrated to `scoutchain_shared_types::U64Page`. Swapping the return type of
/// the existing `get_scout_contacts_page` would change the spec type name in
/// generated bindings (an ABI break). New paged getters should return
/// `U64Page` from shared-types instead of reinventing this shape.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ScoutContactsPage {
    /// Page of contacted player IDs, in contact order (oldest first).
    pub entries: soroban_sdk::Vec<u64>,
    /// Total number of players this scout has contacted.
    pub total: u32,
}

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
    /// Scout wallet that owns this subscription.
    pub scout: Address,
    /// Active subscription tier for authorization and fee checks.
    pub tier: SubscriptionTier,
    /// Ledger timestamp when the subscription expires, in Unix seconds.
    pub expires_at: u64,
    /// Ledger timestamp when the subscription started, in Unix seconds.
    pub subscribed_at: u64,
}

/// A recorded contact event from a scout to a player
#[contracttype]
#[derive(Clone, Debug)]
pub struct ContactRecord {
    /// Player identifier that the scout contacted.
    pub player_id: u64,
    /// Scout wallet that initiated the contact.
    pub scout: Address,
    /// Ledger timestamp at the moment the contact was recorded
    pub contacted_at: u64,
}

/// A logged trial offer from a scout to a player
#[contracttype]
#[derive(Clone, Debug)]
pub struct TrialOffer {
    /// Player identifier receiving the trial offer.
    pub player_id: u64,
    /// Scout wallet that logged the trial offer.
    pub scout: Address,
    /// IPFS/Arweave CID of the offer details document
    pub details_hash: String,
    /// Ledger timestamp when the trial offer was logged, in Unix seconds.
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

/// Escrow record for a trial offer
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TrialEscrow {
    /// Escrowed trial-offer amount in stroops, set from
    /// `FeeConfig.trial_offer_escrow_stroops` when `log_trial_offer` creates
    /// the escrow.
    pub amount: i128,
    /// Ledger timestamp deadline, in Unix seconds, computed as the current
    /// ledger timestamp plus `FeeConfig.trial_offer_expiry_secs`; checked by
    /// `confirm_trial_offer` when deciding whether the trial offer is still
    /// valid.
    pub expires_at: u64,
}

/// Proposed fee configuration awaiting activation after a delay
#[contracttype]
#[derive(Clone, Debug)]
pub struct FeeConfigProposal {
    /// The proposed fee configuration
    pub config: FeeConfig,
    /// Ledger timestamp when the proposal was created
    pub proposed_at: u64,
}

/// Platform fee configuration
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
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
    /// Escrow amount for trial offers (stroops).
    /// Set to 0 to disable trial offers entirely; any positive value
    /// is the amount escrowed per trial offer.
    pub trial_offer_escrow_stroops: i128,
    /// Trial offer expiry window in seconds.
    /// Must be > 0; defines how long an escrowed trial offer remains valid.
    pub trial_offer_expiry_secs: u64,
    /// Maximum contacts per subscription period for Pro tier (default: 10). Resets on renewal.
    /// Elite-tier scouts are exempt from this cap (no limit applies).
    /// See `docs/CONTRACT_REFERENCE.md` — `FeeConfig` and `ProContactLimitReached`
    /// (error 20) for the full per-tier access semantics, and `docs/GLOSSARY.md`
    /// for the definition of "Pro tier" and the contact quota model.
    pub pro_contact_limit: u32,
}

/// A single entry in the bounded on-chain fee configuration history.
/// Stored in `DataKey::FeeConfigHistory` as a `Vec<FeeConfigHistoryEntry>`,
/// oldest-first, capped at `FEE_CONFIG_HISTORY_CAP` entries.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct FeeConfigHistoryEntry {
    /// The fee configuration that was active before this change.
    pub config: FeeConfig,
    /// Ledger timestamp (Unix seconds) when this config was set via `update_fee_config`.
    pub updated_at: u64,
}

/// On-chain proof that `scout` is authorized to request the off-chain
/// wrapped decryption key for `player_id`'s confidential evidence.
///
/// Written atomically by a successful `pay_to_contact` / `batch_contact_players`
/// call (see `docs/EVIDENCE_PRIVACY.md`). This is an append-only fact about
/// the past ("this scout was once entitled to see this evidence"), not a
/// live entitlement check — it is never mutated except by
/// `admin_revoke_evidence_access` or `revoke_evidence_access` flipping
/// `revoked`. Subscription downgrade or expiry does not touch it.
/// Revocation only ever gates *future* off-chain key-wrap requests; it
/// cannot claw back a key that was already delivered before the revoke.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EvidenceAccessGrant {
    /// Player identifier whose evidence this grant authorizes access to.
    pub player_id: u64,
    /// Scout wallet authorized by this grant.
    pub scout: Address,
    /// Ledger timestamp (Unix seconds) when the grant was issued.
    pub granted_at: u64,
    /// Ledger timestamp (Unix seconds) at which the grant expires and is
    /// no longer considered active. `has_evidence_access` returns `false`
    /// once the current ledger time exceeds this value. The grant record
    /// itself is retained (append-only audit trail); only the *live*
    /// entitlement check changes.
    pub expires_at: u64,
    /// The scout's subscription tier at the moment the grant was issued.
    /// Recorded for audit purposes; it is not re-checked afterward.
    pub tier_at_grant: SubscriptionTier,
    /// True once a grant has been revoked via `admin_revoke_evidence_access`
    /// (admin-initiated) or `revoke_evidence_access` (player-initiated).
    /// The record is kept (never deleted) so the audit trail of who was
    /// ever granted access stays intact.
    pub revoked: bool,
    /// Ledger timestamp (Unix seconds) of revocation, if any.
    pub revoked_at: Option<u64>,
}

#[contracttype]
pub enum DataKey {
    Admin,
    /// Proposed replacement admin awaiting acceptance by that address.
    PendingAdmin,
    Initialized,
    Paused,
    /// Function-scoped pause flag for `pay_to_contact` (independent of the
    /// whole-contract `Paused` flag). Stored in instance storage, defaults to
    /// `false` when absent. Mirrors `verification`'s `PausedAppro

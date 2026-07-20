use soroban_sdk::contracterror;
use scoutchain_shared_types::AdminError;

/// Errors for the ScoutAccess contract.
///
/// Append-only: do not renumber existing variants. See docs/CONTRIBUTING.md.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ScoutAccessError {
    // ── Initialization & lifecycle ──
    /// The contract has already been initialized.
    AlreadyInitialized = 1,
    /// The contract has not been initialized.
    NotInitialized = 2,
    /// The contract is currently paused.
    ContractPaused = 3,

    // ── Authorization ──
    /// The caller is not authorized to perform this action.
    Unauthorized = 4,

    // ── Subscription & tier ──
    /// The scout is not subscribed to any tier.
    ScoutNotSubscribed = 6,
    /// The scout's subscription has expired.
    SubscriptionExpired = 7,
    /// The provided subscription tier is invalid.
    InvalidTier = 9,
    /// Scout attempted to downgrade to a cheaper tier while subscription is still active.
    SubscriptionDowngradeNotAllowed = 12,
    /// Scout attempted to upgrade/renew before the minimum interval elapsed.
    UpgradeTooSoon = 17,

    // ── Fees & payments ──
    /// The provided fee is insufficient for the requested action.
    InsufficientFee = 5,
    /// A fee field is zero or negative, or sub_duration_secs is zero.
    InvalidInput = 15,
    /// No accumulated fees available to withdraw.
    NoFeesToWithdraw = 16,

    // ── Contact & trial offers ──
    /// The scout has already contacted this player.
    AlreadyContacted = 8,
    /// The trial offer record was not found.
    TrialOfferNotFound = 11,
    /// Pro tier scout has exceeded monthly contact limit.
    ContactQuotaExceeded = 18,
    /// Scout sent a trial offer to the same player within the cooldown window.
    TrialOfferRateLimited = 19,
    /// Pro-tier scout has reached the contact limit for the current subscription period.
    ProContactLimitReached = 20,

    // ── Cross-contract & arithmetic ──
    /// Arithmetic overflow occurred.
    Overflow = 10,
    // Numeric gap 11 → 14 is intentional (legacy reservation). Do not reuse.
    /// Cross-contract `advance_level` failed.
    ProgressCallFailed = 14,
}

impl AdminError for ScoutAccessError {
    fn not_initialized() -> Self {
        ScoutAccessError::NotInitialized
    }
}

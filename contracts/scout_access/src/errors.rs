use soroban_sdk::contracterror;

/// Errors for the ScoutAccess contract.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ScoutAccessError {
    /// The contract has already been initialized.
    AlreadyInitialized = 1,
    /// The contract has not been initialized.
    NotInitialized = 2,
    /// The contract is currently paused.
    ContractPaused = 3,
    /// The caller is not authorized to perform this action.
    Unauthorized = 4,
    /// The provided fee is insufficient for the requested action.
    InsufficientFee = 5,
    /// The scout is not subscribed to any tier.
    ScoutNotSubscribed = 6,
    /// The scout's subscription has expired.
    SubscriptionExpired = 7,
    /// The scout has already contacted this player.
    AlreadyContacted = 8,
    /// The provided subscription tier is invalid.
    InvalidTier = 9,
    /// Arithmetic overflow occurred.
    Overflow = 10,
    /// The trial offer record was not found.
    TrialOfferNotFound = 11,
    /// Scout attempted to downgrade to a cheaper tier while subscription is still active
    SubscriptionDowngradeNotAllowed = 12,
    ProgressCallFailed = 14,
    /// A fee field is zero or negative, or sub_duration_secs is zero
    InvalidInput = 15,
}

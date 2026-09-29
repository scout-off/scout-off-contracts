use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum VerificationError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    ContractPaused = 3,
    Unauthorized = 4,
    ValidatorNotFound = 5,
    ValidatorInactive = 6,
    ValidatorAlreadyRegistered = 7,
    PlayerNotFound = 8,
    InvalidInput = 9,
    ReasonTooLong = 10,
    AlreadyConfigured = 11,
    ProgressCallFailed = 12,
    Overflow = 13,
    MilestoneNotFound = 14,
    ValidatorCapReached = 15,
    /// A single validator has already approved
    /// MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR milestones for this player.
    MilestoneLimitExceeded = 16,
}

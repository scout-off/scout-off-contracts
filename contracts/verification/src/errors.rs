use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum VerificationError {
    /// `initialize` has already been called on this contract.
    AlreadyInitialized = 1,
    /// A state-changing function was called before `initialize`.
    NotInitialized = 2,
    /// The contract is paused; call `unpause_contract` to resume.
    ContractPaused = 3,
    /// Caller is not the contract admin.
    Unauthorized = 4,
    /// No validator is registered for the given wallet address.
    ValidatorNotFound = 5,
    /// The validator exists but has been revoked; milestones cannot be approved.
    ValidatorInactive = 6,
    /// A validator with this wallet address is already registered.
    ValidatorAlreadyRegistered = 7,
    /// No player profile exists for the given player ID.
    PlayerNotFound = 8,
    /// A required argument is missing, out of range, or otherwise invalid.
    InvalidInput = 9,
    ReasonTooLong = 10,
    AlreadyConfigured = 11,
    ProgressCallFailed = 12,
    Overflow = 13,
    MilestoneNotFound = 14,
}

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ProgressError {
    /// Contract has already been initialized and cannot be initialized again.
    AlreadyInitialized = 1,
    /// Contract has not been initialized yet; call `initialize` first.
    NotInitialized = 2,
    /// Contract is paused; all state-changing operations are blocked.
    ContractPaused = 3,
    /// Caller is not authorized to perform this operation.
    Unauthorized = 4,
    /// The requested level transition is not valid (e.g. skipping a level or going backwards).
    InvalidProgressTransition = 5,
    /// Player is already at the maximum level (EliteTier) and cannot advance further.
    AlreadyAtMaxLevel = 6,
    /// No progress record exists for the given player ID.
    PlayerNotFound = 7,
    /// History counter overflowed the maximum u32 value.
    Overflow = 8,
}

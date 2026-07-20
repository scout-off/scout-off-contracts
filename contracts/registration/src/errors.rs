use soroban_sdk::contracterror;
use scoutchain_shared_types::AdminError;

/// Append-only: do not renumber existing variants. See docs/CONTRIBUTING.md.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ScoutChainError {
    // ── Initialization & lifecycle ──
    /// `initialize` called more than once.
    AlreadyInitialized = 1,
    /// Operation before `initialize`.
    NotInitialized = 2,
    /// Circuit breaker is active.
    ContractPaused = 9,

    // ── Authorization ──
    /// Unregistered account approving milestone.
    ValidatorNotAuthorized = 4,
    /// Wrong account for a privileged operation.
    Unauthorized = 10,

    // ── Registration & lookup ──
    /// Wallet already has a profile for this role.
    AlreadyRegistered = 8,
    /// Invalid `player_id`.
    PlayerNotFound = 3,
    /// Invalid `scout_id`.
    ScoutNotFound = 12,

    // ── Business logic ──
    /// Skipping or reversing a level.
    InvalidProgressTransition = 5,
    /// Scout has no subscription.
    ScoutNotSubscribed = 6,
    /// Underpaying contact fee.
    InsufficientFee = 7,

    // ── Validation & arithmetic ──
    /// Field too long, bad hash count, or empty value.
    InvalidInput = 13,
    /// Counter or fee arithmetic overflowed.
    Overflow = 11,
}

impl AdminError for ScoutChainError {
    fn not_initialized() -> Self {
        ScoutChainError::NotInitialized
    }
}

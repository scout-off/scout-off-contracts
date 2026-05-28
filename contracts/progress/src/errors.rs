use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ProgressError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    ContractPaused = 3,
    Unauthorized = 4,
    InvalidProgressTransition = 5,
    AlreadyAtMaxLevel = 6,
    PlayerNotFound = 7,
    HistoryEntryNotFound = 10,
}

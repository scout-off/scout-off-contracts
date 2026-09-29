use soroban_sdk::{contracttype, Address, Bytes, BytesN, String, Vec};

/// Bound on tracked migration nonces per wallet (documented design limit).
#[allow(dead_code)]
const MAX_MIGRATION_NONCES: u32 = 1024;

pub use scoutchain_shared_types::{ContractHealth, ProgressLevel, WiringLink};

/// Role identifier for migration authorizations.
#[contracttype]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MigrationRole {
    Player,
    Scout,
}

/// An off-chain signed migration authorization produced by a player or scout.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MigrationAuthorization {
    /// Wallet address of the player or scout granting consent.
    pub wallet: Address,
    /// Role being migrated (player or scout).
    pub role: MigrationRole,
    /// Hash of the serialized profile data being authorized for migration.
    pub profile_data_hash: Bytes,
    /// Expected address of the new contract that will redeem this authorization.
    pub new_contract_hint: Address,
    /// Unique nonce to prevent replay. The signer should increment this for
    /// each new authorization they grant.
    pub nonce: u64,
    /// Unix timestamp after which this authorization expires (0 = no expiry).
    pub expires_at: u64,
    /// ed25519 signature over the canonical message:
    /// `wallet || role || profile_data_hash || new_contract_hint || nonce || expires_at`
    pub signature: BytesN<64>,
}

/// Basic player vitals stored on-chain
#[contracttype]
#[derive(Clone, Debug)]
pub struct PlayerVitals {
    /// Player age in years at the time the profile was last written.
    pub age: u32,
    /// Player position label used for discovery filtering.
    pub position: String,
    /// Player region used for scout discovery filtering.
    pub region: String,
    /// Player nationality label displayed in profile results.
    pub nationality: String,
}

/// Internal on-chain player profile (no level — progress contract is the source of truth)
#[contracttype]
#[derive(Clone, Debug)]
pub struct StoredPlayerProfile {
    /// Unique player identifier assigned by the registration contract.
    pub player_id: u64,
    /// Player wallet that owns and can update this profile.
    pub wallet: Address,
    /// Player vitals stored with the profile.
    pub vitals: PlayerVitals,
    /// IPFS/Arweave CIDs for highlight reels and photos
    pub ipfs_hashes: Vec<String>,
    /// Ledger timestamp when the player was first registered, in Unix seconds.
    pub registered_at: u64,
    /// Ledger timestamp when the profile was last updated, in Unix seconds.
    pub updated_at: u64,
}

/// Full on-chain player profile returned to callers.
/// `level` is derived from the progress contract at read time — it is NOT
/// persisted here.  `progress::get_level` is the single source of truth.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PlayerProfile {
    /// Unique player identifier assigned by the registration contract.
    pub player_id: u64,
    /// Player wallet that owns and can update this profile.
    pub wallet: Address,
    /// Player vitals stored with the profile.
    pub vitals: PlayerVitals,
    /// IPFS/Arweave CIDs for highlight reels and photos
    pub ipfs_hashes: Vec<String>,
    /// Current player level loaded from the progress contract at read time.
    pub level: ProgressLevel,
    /// Ledger timestamp when the player was first registered, in Unix seconds.
    pub registered_at: u64,
    /// Ledger timestamp when the profile was last updated, in Unix seconds.
    pub updated_at: u64,
}

/// Lightweight player view for scout discovery (no IPFS hashes or wallet).
#[contracttype]
#[derive(Clone, Debug)]
pub struct PlayerSummary {
    /// Unique player identifier for fetching the full profile.
    pub player_id: u64,
    /// Player vitals exposed for scout discovery.
    pub vitals: PlayerVitals,
    /// Current player level loaded from the progress contract at read time.
    pub level: ProgressLevel,
    /// Ledger timestamp when the profile was last updated, in Unix seconds.
    pub updated_at: u64,
}

/// Paginated response from filter_players.
/// `next_cursor` is `0` when there are no more results.
#[contracttype]
#[derive(Clone, Debug)]
pub struct FilterResult {
    /// Page of player profiles matching the supplied filter criteria.
    pub profiles: Vec<PlayerProfile>,
    /// Pass this value as `offset` in the next call to continue pagination.
    /// A value of `0` means there are no further results.
    pub next_cursor: u64,
}

/// Direct status for a registered player.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerStatus {
    Active,
    Deactivated,
}

/// Direct status for a registered scout.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ScoutStatus {
    Active,
    Deactivated,
    NotRegistered,
}

/// Structured verification record for a scout profile. Replaces/augments the
/// simple `verified: bool` flag with audit evidence so dashboards and future
/// Sybil-mitigation features can consume the verification detail.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ScoutVerificationRecord {
    /// Whether the scout is currently verified.
    pub verified: bool,
    /// Optional admin wallet that performed the verification.
    pub verified_by: Option<Address>,
    /// Ledger timestamp when verification was performed.
    pub verified_at: Option<u64>,
    /// Free-form evidence reference (e.g. KYC provider ID, organization name).
    pub evidence_ref: Option<String>,
    /// Verification method label (e.g. "admin_manual", "kyc_attestation").
    pub method: Option<String>,
}

/// Scout profile stored on-chain
#[contracttype]
#[derive(Clone, Debug)]
pub struct ScoutProfile {
    /// Unique scout identifier assigned by the registration contract.
    pub scout_id: u64,
    /// Scout wallet that owns this profile.
    pub wallet: Address,
    /// Scout operating region used for profile display and discovery context.
    pub region: String,
    /// Legacy boolean flag retained for backward compatibility with existing
    /// consumers. New code should prefer `verification.verified`.
    pub verified: bool,
    /// Structured verification record capturing what was checked, by whom,
    /// when, and evidence reference.
    pub verification: ScoutVerificationRecord,
    /// Ledger timestamp when the scout was registered, in Unix seconds.
    pub registered_at: u64,
}

/// Storage keys for contract state
#[contracttype]
pub enum DataKey {
    /// Admin wallet address authorized to manage validators and fees
    Admin,
    /// Proposed replacement admin. Set by `propose_admin` and removed after
    /// the proposed address proves control by calling `accept_admin`.
    PendingAdmin,
    /// Boolean flag indicating if contract has been initialized
    Initialized,
    /// Boolean flag indicating if contract is paused (circuit breaker)
    Paused,
    /// Counter for generating unique player IDs
    PlayerCounter,
    /// Counter for generating unique scout IDs
    ScoutCounter,
    /// Full player profile stored by player_id
    Player(u64),
    /// Index mapping player wallet address to player_id for fast lookup
    PlayerByWallet(Address),
    /// Full scout profile stored by scout_id
    Scout(u64),
    /// Index mapping scout wallet address to scout_id for fast lookup
    ScoutByWallet(Address),
    /// Index of all player IDs for efficient filtering and iteration
    PlayerIndex,
    /// Address of the progress contract allowed to call set_player_level
    ProgressContract,
    /// Re-wiring epoch for `DataKey::ProgressContract`, bumped by every
    /// `set_progress_contract` call.
    ProgressContractEpoch,
    /// Explicit player level override used for admin-seeded players or
    /// progress updates that should be visible to reads even before a progress
    /// contract is wired.
    PlayerLevel(u64),
    /// Composite index: (ProgressLevel, region) → Vec<u64> of player IDs.
    /// Used by `filter_players` for combined level+region queries so only
    /// matching players are loaded, avoiding a full scan of `PlayerIndex`.
    PlayersByLevelRegion(ProgressLevel, String),
    /// Per-level sub-index: ProgressLevel → Vec<u64> of player IDs.
    /// Primary lookup path for level-filtered queries without a region constraint.
    /// Falls back to `PlayerIndex` only when no level filter is specified.
    PlayersByLevel(ProgressLevel),
    /// Deactivation flag for a player. When present and `true`, the player is
    /// hidden from `filter_players` results while their profile and history are
    /// fully preserved. Set by `deactivate_player`, cleared by `reactivate_player`.
    PlayerDeactivated(u64),

    // ── Registration cooldown ──
    /// Last registration timestamp for a player wallet (Unix seconds).
    /// Set by `register_player` and read to enforce the per-caller cooldown.
    PlayerRegLastSent(Address),
    /// Last registration timestamp for a scout wallet (Unix seconds).
    /// Set by `register_scout` and read to enforce the per-caller cooldown.
    ScoutRegLastSent(Address),
    /// Last registration timestamp for a validator wallet (Unix seconds).
    /// Set by `register_validator` in the verification contract; mirrored here
    /// via the same DataKey convention for cross-contract inspection.
    ValidatorRegLastSent(Address),
    /// Cooldown in seconds between repeated registration attempts from the
    /// same wallet. 0 means no cooldown. Configurable by admin.
    RegCooldownSecs(u64), // ── Migration ticket replay prevention ──
    /// Nonce tracking for migration authorizations. A wallet+nonce pair is
    /// stored as `true` after a migration authorization is redeemed, preventing
    /// the same authorization from being replayed.
    MigrationNonce(Address, u64),

    // ── Scout deactivation ──
    /// Deactivation flag for a scout. When present and `true`, the scout is
    /// hidden from scout discovery results. Set by `deactivate_scout`,
    /// cleared by `reactivate_scout`.
    ScoutDeactivated(u64),
}

/// Snapshot of the single cross-contract peer address pointer held by the
/// registration contract (progress), with its address and re-wiring epoch.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RegistrationWiringState {
    pub progress_contract: WiringLink,
}

impl RegistrationWiringState {
    pub fn is_fully_wired(&self) -> bool {
        self.progress_contract.is_configured()
    }
}

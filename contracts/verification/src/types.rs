pub use scoutchain_shared_types::{ContractHealth, WiringLink};
use soroban_sdk::{contracttype, Address, BytesN, String, Vec};

/// Convenience aggregate returned by `get_validator_activity_report`.
///
/// Bundles the data from four individual queries into one call:
/// - `get_validator`               → credentials, registered_at, active
/// - `get_validator_status`        → status
/// - `get_validator_milestone_count` → milestone_count
/// - `get_validator_players`       → distinct_players (and distinct_player_count)
///
/// This is a pure read-only aggregate — no new storage or business logic.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatorActivityReport {
    /// Validator wallet address.
    pub wallet: Address,
    /// Human-readable credential label set at registration time.
    pub credentials: String,
    /// Unix timestamp (seconds) when the validator was registered.
    pub registered_at: u64,
    /// Whether the validator is currently active.
    pub active: bool,
    /// Richer status distinguishing Active / Revoked / RevokedForCause / NotRegistered.
    pub status: ValidatorStatus,
    /// Total number of milestones approved by this validator across all players.
    pub milestone_count: u32,
    /// Number of distinct players for whom this validator has approved at least one milestone.
    pub distinct_player_count: u32,
    /// List of distinct player IDs (same data as `get_validator_players`).
    pub distinct_players: Vec<u64>,
}

/// Richer validator status — distinguishes unregistered from revoked.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ValidatorStatus {
    NotRegistered,
    Active,
    Revoked,
    RevokedForCause,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MilestoneWithValidatorStatus {
    /// Milestone record returned with validator status context.
    pub milestone: Milestone,
    /// Current status of the validator that approved the milestone.
    pub validator_status: ValidatorStatus,
}

/// A single verified milestone record
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Milestone {
    /// Unique player identifier this milestone belongs to.
    pub player_id: u64,
    /// Validator wallet that approved the milestone.
    pub validator: Address,
    /// Human-readable milestone description.
    pub description: String,
    /// IPFS/Arweave CID of supporting evidence (video clip, stat sheet, etc.)
    pub evidence_hash: String,
    /// Ledger timestamp when the milestone was approved, in Unix seconds.
    pub approved_at: u64,
    /// Stellar ledger sequence at time of approval for tamper-proof auditability
    pub ledger_sequence: u32,
}

/// Validator entry in the trusted registry
#[contracttype]
#[derive(Clone, Debug)]
pub struct Validator {
    /// Validator wallet authorized to approve milestones.
    pub wallet: Address,
    /// Human-readable credential label (e.g. "UEFA B License", "Academy Director")
    pub credentials: String,
    /// Administrator-verified organizational affiliation (e.g. "FC Example Academy")
    pub affiliation: String,
    /// Ledger timestamp when the validator was registered, in Unix seconds.
    pub registered_at: u64,
    /// Whether this validator is currently authorized to approve milestones.
    pub active: bool,
    /// Optional specialization tags (e.g. "physical-stats", "identity-kyc", "match-performance").
    /// When a milestone category is provided to `approve_milestone`, only validators with a
    /// matching specialization tag can approve it. An empty Vec means the validator can approve
    /// any untagged (general-category) milestone but cannot approve tagged milestones.
    pub specializations: Vec<String>,
    /// Geographic region (e.g. "West Africa", "Europe") used by the
    /// `min_region_quorum` anti-collusion gate: approving validators must span
    /// at least that many distinct regions before a gated level (2/3) advance
    /// commits (see `set_min_region_quorum` / README Progress Level table).
    /// APPENDED LAST to keep existing stored `Validator` entries
    /// deserializable after upgrade (safe-layout-compat, matching
    /// `scripts/fixtures/storage_compat_safe/new_types.rs`).
    pub region: String,
}

/// Entry in the global milestone index for on-chain auditability.
#[contracttype]
#[derive(Clone, Debug)]
pub struct GlobalMilestoneEntry {
    /// Unique player identifier for the indexed milestone.
    pub player_id: u64,
    /// Per-player milestone index for fetching the full milestone.
    pub milestone_index: u32,
}

/// Paginated response for global milestone index queries.
#[contracttype]
#[derive(Clone, Debug)]
pub struct GlobalMilestoneIndexPage {
    /// Page of global milestone index entries.
    pub entries: Vec<GlobalMilestoneEntry>,
    /// Total number of milestones in the global index.
    pub total: u32,
}

/// Paginated response for `get_validator_milestones_page` and the new
/// `get_validator_players_page`.
///
/// Follows the same `{ entries, total }` convention as
/// [`GlobalMilestoneIndexPage`] so callers have a consistent stop condition.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MilestoneRefPage {
    /// Page of milestone references, in approval order (oldest first).
    pub entries: Vec<MilestoneRef>,
    /// Total number of milestone references in the validator's list.
    pub total: u32,
}

/// Paginated response for `get_validator_players_page`.
///
/// `entries` is a page of distinct player IDs for which the validator has
/// approved at least one milestone; `total` is the size of the full list.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatorPlayersPage {
    /// Page of distinct player IDs, in order of first approval.
    pub entries: Vec<u64>,
    /// Total number of distinct players in the validator's list.
    pub total: u32,
}

/// A player-initiated dispute for a milestone.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MilestoneDispute {
    /// Unique player identifier for the disputed milestone.
    pub player_id: u64,
    /// Per-player milestone index being disputed.
    pub milestone_index: u32,
    /// Player-provided dispute reason.
    pub reason: String,
    /// Ledger timestamp when the dispute was opened, in Unix seconds.
    pub disputed_at: u64,
    /// Whether the dispute has been resolved.
    pub resolved: bool,
    /// Whether the dispute was upheld when resolved.
    pub upheld: bool,
    /// Impact score supplied when the dispute was filed.
    pub impact_score: u32,
    /// Whether this dispute requires a jury vote (impact_score >= jury threshold at filing time).
    pub jury_required: bool,
    /// Minimum votes required for a jury outcome (snapshotted at filing time).
    pub quorum: u32,
    /// Unix timestamp when the voting window closes (snapshotted at filing time).
    pub voting_deadline: u64,
    /// Number of votes cast in favour of upholding the dispute.
    pub votes_for: u32,
    /// Number of votes cast against upholding the dispute.
    pub votes_against: u32,
}

/// Admin-configurable jury parameters for high-impact milestone disputes.
/// Defaults: impact_threshold = 100, quorum = 3, voting_window_secs = 604800 (7 days).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct JuryConfig {
    /// Minimum impact score for a dispute to require jury resolution.
    pub impact_threshold: u32,
    /// Minimum number of distinct validator votes required for a jury outcome.
    pub quorum: u32,
    /// Duration in seconds from filing during which validators may vote.
    pub voting_window_secs: u64,
}

/// A single validator vote on a jury-required dispute.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DisputeVote {
    /// Validator wallet that cast this vote.
    pub validator: Address,
    /// Whether the validator voted to uphold the dispute.
    pub for_upheld: bool,
    /// Unix timestamp when the vote was cast.
    pub voted_at: u64,
}

/// A lightweight reference to a milestone (player + index).
/// Stored in `DataKey::ValidatorMilestones` as a compact per-validator index.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MilestoneRef {
    /// Unique player identifier for the referenced milestone.
    pub player_id: u64,
    /// Per-player milestone index.
    pub milestone_index: u32,
}

/// Off-chain signed milestone attestation (issue #703).
///
/// Canonical signed message (domain-separated):
/// `ATTESTATION_DOMAIN || contract_id || network_id || validator_wallet
///  || player_id_be || description_bytes || evidence_hash_bytes || nonce_be`
///
/// Field rationale:
/// - `validator_wallet`: binds the claim to a registry identity; after signature
///   verification against that wallet's registered pubkey, this is the sole
///   source of attribution (never a separate caller-supplied Address).
/// - `player_id` / `description` / `evidence_hash`: exact claim being attested.
/// - `nonce`: strictly-increasing per-validator counter for replay protection
///   (raw ed25519 signatures have no Soroban sequence number).
/// - `contract_id` + `network_id`: prevent cross-deployment / cross-network replay.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MilestoneAttestation {
    /// Validator whose registered attestation key must have signed this payload.
    pub validator_wallet: Address,
    /// Player receiving the milestone.
    pub player_id: u64,
    /// Human-readable milestone description.
    pub description: String,
    /// IPFS/Arweave CID of supporting evidence.
    pub evidence_hash: String,
    /// Strictly increasing per-validator nonce (must be > last accepted).
    pub nonce: u64,
    /// Must equal `env.current_contract_address()` at verification time.
    pub contract_id: Address,
    /// Must equal `env.ledger().network_id()` at verification time.
    pub network_id: BytesN<32>,
}

/// Bounded, fixed-size accumulator for a k-of-n milestone attestation claim
/// (issue: threshold milestone approval). Keyed by canonical claim identity
/// (player_id, evidence_hash) — see `attest_milestone` for why description
/// text is intentionally excluded from the identity.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingMilestoneClaim {
    pub player_id: u64,
    pub evidence_hash: String,
    /// Description locked in by the first attestation of this (player_id,
    /// evidence_hash, round). Later voters' description text does not
    /// overwrite it, so the threshold-reaching validator cannot rewrite the
    /// claim's narrative at the last moment.
    pub description: String,
    /// Distinct, currently-valid active-validator votes accumulated so far
    /// in this round.
    pub vote_count: u32,
    /// Bumped on every voting-window expiry; invalidates all prior votes
    /// without touching their storage — see `DataKey::PendingMilestoneVote`.
    pub round: u32,
    /// Ledger timestamp (Unix seconds) this round started.
    pub created_at: u64,
    /// Threshold snapshotted when this round started, so an admin changing
    /// the global threshold mid-vote cannot retroactively fast-track or
    /// invalidate an in-flight claim.
    pub threshold: u32,
    /// Per-(region, count) tally of the distinct validator regions among the
    /// attesters who have voted in this round. `count` tracks how many
    /// active votes carry that region so a later `revoke_validator` can
    /// decrement it exactly (the claim's stored vote count and this tally
    /// stay in lockstep). At threshold-crossing the region-quorum gate is
    /// evaluated over the entries with `count > 0` — the attesting validator
    /// set — NOT the accumulated per-player region history.
    /// APPENDED LAST (safe-layout-compat): in-flight claims written by a
    /// pre-upgrade build are transient (TTL-bounded, cleared on commit or
    /// round expiry), so a decode failure here only orphans short-lived
    /// pending-vote state, never committed milestones.
    pub attester_regions: Vec<(String, u32)>,
    /// Per-(affiliation, count) tally of the distinct validator affiliations
    /// among this round's attesters, mirroring `attester_regions` for the
    /// `DiversityConfig` affiliation gate.
    pub attester_affiliations: Vec<(String, u32)>,
}

/// Distinct regions/affiliations of the validators whose votes reached the
/// k-of-n threshold for a single claim (derived from
/// `PendingMilestoneClaim::attester_regions` / `attester_affiliations` at
/// threshold-crossing).
///
/// The `min_region_quorum` and `DiversityConfig` affiliation gates evaluate
/// against THIS set for k-of-n approvals, so a threshold met entirely by
/// validators from one region or one affiliation can never advance the level
/// on the strength of accumulated history from other milestones. The
/// single-validator paths (`approve_milestone`,
/// `submit_attested_milestone`, and `attest_milestone` with threshold == 1)
/// pass `None` and keep evaluating against the accumulated per-player sets.
#[derive(Clone, Debug, PartialEq)]
pub struct AttesterDiversity {
    /// Distinct validator regions among the attesting set.
    pub regions: Vec<String>,
    /// Distinct validator affiliations among the attesting set.
    pub affiliations: Vec<String>,
}

/// Reference to one of a validator's currently-open pending-claim votes.
/// Stored (bounded, capped) under `DataKey::ValidatorPendingVotes` purely so
/// `revoke_validator` can find and retract this validator's contribution to
/// any still-pending claim without an unbounded storage scan.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingVoteRef {
    pub player_id: u64,
    pub evidence_hash: String,
    pub round: u32,
}

/// Result of `attest_milestone` — whether this vote just crossed threshold.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum AttestationStatus {
    /// Vote recorded; still short of threshold. Payload is the new vote count.
    Pending(u32),
    /// This vote reached threshold; the milestone was committed and
    /// `progress.advance_level` was invoked — unless the region-quorum or
    /// affiliation-diversity gates blocked the advance (see
    /// `commit_approved_milestone`), in which case the milestone is recorded
    /// but the level is not advanced. Payload is the milestone index.
    Committed(u32),
}

/// Severity level for a validator revocation.
///
/// Passed explicitly to `revoke_validator` instead of inferring severity from
/// free-text string content.  `Routine` leaves prior milestone approvals
/// untouched; `ForCause` triggers a cascade sweep that flags every milestone
/// the validator previously approved as `MilestonePendingReReview`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum RevocationSeverity {
    /// Routine deactivation — no cascade flag sweep triggered.
    Routine,
    /// For-cause revocation — every previously-approved milestone is flagged
    /// `MilestonePendingReReview` so scouts and indexers can identify affected
    /// records.
    ForCause,
}

/// Persisted record of a validator revocation (stored under
/// `DataKey::RevocationRecord(wallet)`).
///
/// Retains the severity, human-readable reason, ledger timestamp, and the
/// admin address that performed the revocation for audit purposes.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RevocationRecord {
    /// Severity of this revocation.
    pub severity: RevocationSeverity,
    /// Human-readable reason supplied by the admin (may be empty).
    pub reason: String,
    /// Unix timestamp (seconds) when the revocation occurred.
    pub revoked_at: u64,
    /// Admin address that performed the revocation.
    pub admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DiversityConfig {
    pub required_distinct_affiliations: u32,
    pub starting_milestone_index: u32,
}

#[contracttype]
pub enum DataKey {
    Admin,
    /// Proposed replacement admin awaiting acceptance by that address.
    PendingAdmin,
    Initialized,
    Paused,
    /// Function-scoped pause flag for approve_milestone (independent of whole-contract Paused)
    PausedApproveMilestone,
    ProgressContract,
    ProgressContractSet,
    Validator(Address),
    MilestoneCounter(u64),
    Milestone(u64, u32),
    ValidatorMilestoneCount(Address),
    ValidatorPlayerMilestoneCount(Address, u64),
    ValidatorVector,
    TotalMilestoneCount,
    /// Reserved legacy key for the pre-ring global milestone index. New reads
    /// and writes use `GlobalMilestoneWriteHead` + `GlobalMilestoneSlot(slot)`.
    GlobalMilestoneIndex,
    /// Monotonic write counter for the ring-buffer global milestone index.
    /// Stored in instance storage. Value is the total number of entries ever
    /// written (including evicted ones). O(1) to read and update.
    GlobalMilestoneWriteHead,
    /// One slot of the ring-buffer global milestone index, stored in
    /// **persistent** storage (not instance) so individual slots have
    /// per-key TTL management and the hot instance-storage entry does not
    /// balloon with each slot write.
    ///
    /// `slot_index = write_head % MAX_GLOBAL_MILESTONE_INDEX`
    GlobalMilestoneSlot(u32),
    /// Persistent config for diversity-gated milestone advancement
    DiversityConfig,
    /// Persistent index: player_id → Vec<String> distinct affiliations that have contributed milestones
    PlayerAffiliations(u64),
    /// Persistent index: player_id → Vec<String> distinct validator REGIONS that
    /// have contributed milestones, used by the `min_region_quorum` gate for the
    /// single-validator approval paths. Mirrors `PlayerAffiliations` so both
    /// anti-collusion predicates are evaluated over the same accumulated validator
    /// diversity for a player. k-of-n threshold approvals instead evaluate the
    /// gate over the attesting validator set (see `AttesterDiversity`).
    PlayerRegions(u64),
    /// Persistent index: validator wallet → Vec<u64> of distinct player_ids
    /// for which that validator has approved at least one milestone.
    /// Updated on every `approve_milestone` call (duplicates are skipped).
    ValidatorPlayers(Address),
    MilestoneDispute(u64, u32),
    ActiveValidatorCount,
    TotalValidatorCount,
    /// Evidence hash → (player_id, milestone_index) for global uniqueness and usage lookup.
    EvidenceUsed(String),
    ValidatorMilestones(Address),
    ActiveDisputesCount,
    ValidatorRevokedForCause(Address),
    /// Per-player list of milestone indices that have been disputed.
    /// player_id → Vec<u32> of milestone_index values.
    /// Updated on `dispute_milestone`.
    PlayerDisputes(u64),
    /// Persistent global index of currently-unresolved (player_id, milestone_index) pairs.
    /// Populated on `dispute_milestone`, pruned on `resolve_dispute`.
    /// Exposed via `list_disputes_page(offset, limit)`.
    OpenDisputeIndex,

    // ── Registration cooldown ──
    /// Last registration timestamp for a validator wallet (Unix seconds).
    /// Set by `register_validator` and read to enforce the per-caller cooldown.
    ValidatorRegLastSent(Address),
    /// Platform-wide validator registration cooldown in seconds.
    /// 0 disables the cooldown. Configurable by admin via `set_reg_cooldown`.
    /// The `u64` payload is unused (always written as `RegCooldownSecs(0)`).
    RegCooldownSecs(u64),
    /// Minimum distinct validator regions required for Level-2/3 advances.
    MinRegionQuorum,
    /// Validator wallet → registered ed25519 public key (32 bytes) for
    /// off-chain milestone attestation verification.
    AttestationKey(Address),
    /// Reverse index: attestation pubkey → validator wallet. Used so identity
    /// is derived from the verified key, not from a caller-supplied Address.
    AttestationKeyOwner(BytesN<32>),
    /// Per-validator monotonic nonce for relayed attestation replay protection.
    /// Stores the last successfully consumed nonce (starts absent → treat as 0).
    AttestationNonce(Address),

    // ── k-of-n threshold milestone attestation ──
    /// Pending (sub-threshold) milestone attestation accumulator, keyed by
    /// the canonical claim identity (player_id, evidence_hash). See
    /// `attest_milestone`.
    PendingMilestoneClaim(u64, String),
    /// One validator's vote on a specific (player_id, evidence_hash, round).
    /// `round` is bumped whenever a sub-threshold claim expires, which makes
    /// every vote cast in a prior round unreachable without needing to
    /// delete or enumerate it — see `attest_milestone` for the expiry
    /// mechanism.
    PendingMilestoneVote(u64, String, u32, Address),
    /// Bounded list (capped at MAX_PENDING_VOTES_PER_VALIDATOR) of claims a
    /// validator currently has an open, uncommitted, unexpired vote on. Used
    /// solely so `revoke_validator` can retroactively invalidate that
    /// validator's contribution to any still-pending claim without an
    /// unbounded storage scan.
    ValidatorPendingVotes(Address),
    /// k-of-n distinct-active-validator threshold required before a
    /// milestone claim accumulated via `attest_milestone` is committed.
    /// Defaults to 1 — see `get_milestone_threshold`.
    MilestoneApprovalThreshold,
    /// Voting window (seconds) within which `threshold` distinct votes must
    /// accumulate before a claim expires. See `get_voting_window_secs`.
    AttestationVotingWindowSecs,

    // ── Registration cross-contract (issue #1014) ──
    /// Address of the registration contract used to verify wallet↔player_id binding.
    RegistrationContract,
    /// Whether `RegistrationContract` has been set at least once.
    RegistrationContractSet,

    /// Boolean flag (`true`) written by `open_migration_window`; absent or
    /// `false` means the migration window is closed. All `admin_seed_*`
    /// functions on this contract check this flag before writing any state.
    /// Cleared by `close_migration_window`. Stored in instance storage.
    MigrationActive,

    // ── Wiring epochs (issue #1041) ──
    /// Re-wiring epoch for `DataKey::ProgressContract`, bumped by every
    /// `set_progress_contract` / `update_progress_contract` call. See
    /// `scoutchain_shared_types::WiringLink` and
    /// `docs/WIRING_REGISTRY_DESIGN.md`.
    ProgressContractEpoch,
    /// Re-wiring epoch for `DataKey::RegistrationContract`, bumped by every
    /// `set_registration_contract` / `update_registration_contract` call.
    RegistrationContractEpoch,

    // ── Jury escalation system (issue #1036) ──
    /// Admin-configurable jury parameters stored in instance storage.
    /// Defaults: impact_threshold=100, quorum=3, voting_window_secs=604800.
    JuryConfig,
    /// Individual validator vote on a jury-required dispute.
    /// Keyed by (player_id, milestone_index, validator_wallet).
    DisputeVote(u64, u32, Address),
    /// Running vote count for a dispute, keyed by (player_id, milestone_index).
    /// Provides an O(1) count without scanning individual DisputeVote entries.
    DisputeVoteCount(u64, u32),

    // ── Validator revocation cascade re-review (issue #1039) ──
    /// Persisted `RevocationRecord` for a revoked validator wallet.
    /// Keyed by validator wallet address.
    RevocationRecord(Address),
    /// Legacy per-milestone pending-re-review flag. New cascades use the
    /// validator-scoped pages below so a bounded batch does not exceed the
    /// network's per-transaction ledger-write limit; readers retain support
    /// for this key for state written by an earlier implementation.
    MilestonePendingReReview(u64, u32),
    /// Sharded pending-re-review flags for one revoked validator. Each page
    /// contains compact `(player_id, milestone_index)` references.
    MilestonePendingReReviewPage(Address, u32),
    /// Number of pending-re-review references stored for one revoked validator.
    MilestonePendingReReviewCount(Address),
    /// Continuation cursor for a bounded for-cause cascade sweep.
    /// Stores the index (0-based position in the `ValidatorMilestones` vec)
    /// from which the next `continue_revocation_cascade` call should resume.
    /// Absent when no cascade is in progress or when the sweep is complete.
    RevocationCascadeCursor(Address),
}

/// Snapshot of both cross-contract peer address pointers held by the
/// verification contract, each with its address and re-wiring epoch.
/// Returned by [`VerificationContract::get_wiring_state`].
///
/// See `docs/WIRING_REGISTRY_DESIGN.md` for the full cross-contract picture
/// and `scoutchain_shared_types::WiringLink` for what `epoch` means.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VerificationWiringState {
    /// Peer link to the progress contract. Set via `set_progress_contract`
    /// (first call only — see `DataKey::ProgressContractSet`) or
    /// `update_progress_contract` (any subsequent call). Only this address
    /// may be the target of `advance_level` cross-calls from
    /// `approve_milestone` / `attest_milestone`.
    pub progress_contract: WiringLink,
    /// Peer link to the registration contract. Set via
    /// `set_registration_contract` (first call only — see
    /// `DataKey::RegistrationContractSet`) or `update_registration_contract`.
    /// Used by `dispute_milestone` to verify wallet↔player_id binding.
    pub registration_contract: WiringLink,
}

impl VerificationWiringState {
    /// Returns `true` iff both peer links are configured.
    pub fn is_fully_wired(&self) -> bool {
        self.progress_contract.is_configured() && self.registration_contract.is_configured()
    }
}

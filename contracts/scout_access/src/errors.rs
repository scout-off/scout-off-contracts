use scoutchain_shared_types::AdminError;
use soroban_sdk::contracterror;

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
    // Code 13 is intentionally reserved and must not be reassigned. It was
    // never assigned to a live variant but is held open to prevent future
    // contributors from accidentally colliding with any external consumers
    // that may already treat 13 as an expected (if undocumented) gap.
    // See docs/VERSIONING.md — error-code compatibility.
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
    ///
    /// DEPRECATED: this error code is no longer returned by any contract
    /// function. It is retained here only to reserve slot 18 and prevent
    /// accidental reassignment, matching the code-13 reservation pattern.
    /// Callers should handle `ProContactLimitReached` (20) instead.
    ContactQuotaExceeded = 18,
    /// Scout sent a trial offer to the same player within the cooldown window.
    TrialOfferRateLimited = 19,
    /// Pro-tier scout has reached the contact limit for the current subscription period.
    ProContactLimitReached = 20,
    /// The trial offer has already been confirmed.
    TrialOfferAlreadyConfirmed = 22,
    /// Legacy compatibility code for an expired offer. The current expiry
    /// path commits the refund and returns `Ok(())` because returning this
    /// error would roll back the refund under Soroban transaction semantics.
    TrialOfferExpired = 23,

    // ── Cross-contract & arithmetic ──
    /// Arithmetic overflow occurred.
    Overflow = 10,
    /// Cross-contract `advance_level` failed.
    ProgressCallFailed = 14,

    // ── Admin transfer ──
    /// `accept_admin` called before an admin transfer was proposed.
    PendingAdminNotSet = 21,

    // ── Fee config proposal ──
    /// No pending fee config proposal exists for `activate_fee_config`.
    NoPendingFeeConfig = 24,
    /// Pending fee config proposal activation delay has not yet elapsed.
    FeeConfigProposalNotReady = 25,
    /// A fee config proposal already exists; must activate or replace it.
    PendingFeeConfigAlreadyExists = 26,

    // ── Sybil resistance ──
    /// Scout is not verified; cannot subscribe to Pro tier.
    ScoutNotVerified = 27,

    // ── Auto-renewal ──
    /// `renew_if_due` was called but auto-renewal is not enabled for this scout.
    AutoRenewNotEnabled = 28,

    // ── Migration ──
    /// Migration window is not currently active on this contract.
    /// Call `open_migration_window` (admin-only) before seeding state.
    MigrationNotActive = 29,
    /// A `Subscription` already exists for this scout address with different
    /// content. Identical replays are no-ops; conflicting replays are rejected.
    SubscriptionAlreadyExists = 30,
    /// A `ContactRecord` already exists for `(player_id, scout)` with different
    /// content. Identical replays are no-ops.
    ContactAlreadyExists = 31,
    /// A `TrialOffer` already exists at `(player_id, trial_index)` with
    /// different content. Identical replays are no-ops.
    TrialOfferAlreadyExists = 32,
    /// An `AutoRenew` flag already exists for this scout with a different value.
    /// Identical replays are no-ops.
    AutoRenewAlreadyExists = 33,
    /// A fee-config history replay conflicts with history already stored.
    FeeConfigHistoryAlreadyExists = 34,
    /// `restore_subscription_record` targeted a subscription entry whose
    /// archival grace period has fully elapsed (evicted, not merely archived)
    /// and is unrecoverable. (Codes 13 and 18 are reserved by tests.)
    SubscriptionRecordEvicted = 35,

    // ── Function-scoped pausing ──
    /// The `pay_to_contact` function is paused independently of the
    /// whole-contract pause (issue #1056). Mirrors `verification`'s
    /// `ApproveMilestonePaused`.
    PayToContactPaused = 36,

    // ── Trial escrow rescue ──
    /// `admin_refund_trial_escrow` targeted a `(player_id, offer_index)` pair
    /// with no outstanding `TrialEscrow` entry: it was never created, or was
    /// already released by `confirm_trial_offer` or `expire_trial_offers`
    /// (or by a prior `admin_refund_trial_escrow` call).
    TrialEscrowNotOutstanding = 37,

    // ── Evidence access grants ──
    /// `admin_revoke_evidence_access` targeted a (player_id, scout) pair for
    /// which no `EvidenceAccessGrant` record exists.
    GrantNotFound = 38,
}

impl AdminError for ScoutAccessError {
    fn not_initialized() -> Self {
        ScoutAccessError::NotInitialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scout_access_error_code_13_remains_reserved() {
        let mut in_error_enum = false;
        let mut next_implicit_code: Option<u32> = None;

        for raw_line in include_str!("errors.rs").lines() {
            let line = raw_line.trim();

            if line.starts_with("pub enum ScoutAccessError") {
                in_error_enum = true;
                continue;
            }

            if !in_error_enum {
                continue;
            }

            if line.starts_with('}') {
                break;
            }

            if line.is_empty()
                || line.starts_with("//")
                || line.starts_with("///")
                || !line.ends_with(',')
            {
                continue;
            }

            let assigned_code = if let Some((_, discriminant)) = line.split_once('=') {
                discriminant
                    .trim_end_matches(',')
                    .trim()
                    .parse::<u32>()
                    .expect("ScoutAccessError discriminants must be u32 literals")
            } else {
                next_implicit_code.expect("first ScoutAccessError variant must be explicit")
            };

            assert_ne!(
                assigned_code, 13,
                "ScoutAccessError code 13 is intentionally reserved and must not be assigned"
            );

            next_implicit_code = Some(assigned_code + 1);
        }
    }

    #[test]
    fn contact_quota_exceeded_is_deprecated_slot_18_reserved() {
        assert_eq!(ScoutAccessError::ContactQuotaExceeded as u32, 18);
    }

    #[test]
    fn pro_contact_limit_reached_is_code_20() {
        assert_eq!(ScoutAccessError::ProContactLimitReached as u32, 20);
    }

    #[test]
    fn grant_not_found_is_code_38() {
        assert_eq!(ScoutAccessError::GrantNotFound as u32, 38);
    }
}

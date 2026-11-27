use scoutchain_shared_types::ProgressLevel;

use crate::fixtures::Harness;

/// Numeric rank of a progress tier (0..=3). Used to detect decreases and skips.
pub fn level_rank(level: &ProgressLevel) -> u32 {
    match level {
        ProgressLevel::Unverified => 0,
        ProgressLevel::VerifiedIdentity => 1,
        ProgressLevel::PerformanceMilestones => 2,
        ProgressLevel::EliteTier => 3,
    }
}

fn is_valid_level(level: &ProgressLevel) -> bool {
    matches!(
        level,
        ProgressLevel::Unverified
            | ProgressLevel::VerifiedIdentity
            | ProgressLevel::PerformanceMilestones
            | ProgressLevel::EliteTier
    )
}

/// True when `new` is exactly one tier above `old` (the only legal non-reset step).
fn is_forward_step(old: &ProgressLevel, new: &ProgressLevel) -> bool {
    match old.next() {
        Some(expected) => expected == *new,
        None => false,
    }
}

/// Invariant 1: Fee conservation
///
/// `scout_access.get_accumulated_fees()` must:
///   * be non-negative,
///   * equal the harness's running total of fee-generating events minus
///     withdrawals / refunds (`expected_fees`),
///   * never have decreased except as a result of an explicit
///     withdraw/refund recorded by the harness.
///
/// Token transfers are real in this harness (a test SAC is minted and
/// transferred), so the counter is checked against the operations the
/// fuzzer actually executed rather than treated as a no-op.
pub fn assert_fee_conservation(harness: &Harness) -> Result<(), String> {
    let actual = harness.scout_access.get_accumulated_fees();

    if actual < 0 {
        return Err(format!("accumulated fees are negative: {actual}"));
    }

    if harness.fee_counter_regressed {
        return Err(format!(
            "accumulated-fee counter decreased without a matching withdraw/refund \
             (last observed {}, expected {})",
            harness.last_observed_fees, harness.expected_fees
        ));
    }

    if actual != harness.expected_fees {
        return Err(format!(
            "fee conservation violated: get_accumulated_fees()={actual}, \
             expected {} (withdrawn/refunded {})",
            harness.expected_fees, harness.fees_withdrawn_or_refunded
        ));
    }

    // Without an explicit withdraw/refund the counter is monotonically
    // non-decreasing from the value snapshotted at the start of the schedule.
    if harness.fees_withdrawn_or_refunded == 0 && actual < harness.last_observed_fees {
        return Err(format!(
            "fee counter regressed from {} to {actual} with no withdraw/refund",
            harness.last_observed_fees
        ));
    }

    Ok(())
}

/// Invariant 2: Level monotonicity
///
/// For every player known to the harness:
///   * `get_level` returns a valid `ProgressLevel`,
///   * `get_progress_history` is a valid transition chain from Unverified
///     (each step is either +1 tier, or an explicit reset marked by
///     `milestone_ref == 0`, which is how `reset_player_level` records
///     itself — matching the `player_level_reset` event),
///   * the current `get_level` matches the last history entry (or Unverified
///     when history is empty),
///   * the current level did not decrease or skip relative to the
///     previously-observed level unless a reset appears in history.
pub fn assert_level_monotonicity(harness: &Harness) -> Result<(), String> {
    for &player_id in &harness.player_ids {
        let current = harness.progress.get_level(&player_id);
        if !is_valid_level(&current) {
            return Err(format!("invalid level for player {player_id}: {current:?}"));
        }

        let history = harness.progress.get_progress_history(&player_id);
        let mut cursor = ProgressLevel::Unverified;
        let mut saw_reset = false;

        for i in 0..history.len() {
            let entry = history.get(i).unwrap();
            if entry.player_id != player_id {
                return Err(format!(
                    "history entry for player {player_id} carries player_id {}",
                    entry.player_id
                ));
            }
            if entry.old_level != cursor {
                return Err(format!(
                    "player {player_id} history[{i}] old_level={:?} does not continue \
                     from previous new_level={cursor:?}",
                    entry.old_level
                ));
            }

            // `reset_player_level` records milestone_ref = 0 and emits
            // `player_level_reset`. That is the only legal way to decrease
            // or skip a tier.
            let is_reset = entry.milestone_ref == 0;
            if is_reset {
                saw_reset = true;
            } else if !is_forward_step(&entry.old_level, &entry.new_level) {
                return Err(format!(
                    "player {player_id} history[{i}] is not a +1 advance and not a reset: \
                     {:?} → {:?} (milestone_ref={})",
                    entry.old_level, entry.new_level, entry.milestone_ref
                ));
            }
            cursor = entry.new_level.clone();
        }

        if history.is_empty() {
            if current != ProgressLevel::Unverified {
                return Err(format!(
                    "player {player_id} has empty history but get_level()={current:?}"
                ));
            }
        } else if current != cursor {
            return Err(format!(
                "player {player_id} get_level()={current:?} does not match last \
                 history new_level={cursor:?}"
            ));
        }

        if let Some(prior) = harness.last_observed_levels.get(&player_id) {
            let prior_r = level_rank(prior);
            let now_r = level_rank(&current);
            if now_r < prior_r && !saw_reset {
                return Err(format!(
                    "player {player_id} level decreased {prior:?} → {current:?} \
                     without a reset_player_level / player_level_reset history entry"
                ));
            }
            if now_r > prior_r + 1 && !saw_reset {
                // A skip from the previously observed snapshot is only legal
                // if history recorded the intermediate +1 steps (already
                // checked above) — so this is a "jumped without history"
                // case, which the chain walk would also have caught if the
                // history itself skipped. Re-state it against the snapshot
                // so a missing intermediate observation is explicit.
                let steps = now_r - prior_r;
                let history_advances = history.len();
                if history_advances < steps {
                    return Err(format!(
                        "player {player_id} skipped {steps} tiers from {prior:?} \
                         to {current:?} with only {history_advances} history entries \
                         and no reset"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Invariant 3: Validator consistency
/// Every validator referenced by the harness is (or was, if later revoked)
/// a real registered validator.
pub fn assert_validator_consistency(harness: &Harness) -> Result<(), String> {
    for i in 0..harness.validators.len() {
        let validator = harness.validators.get(i).unwrap();
        let status = harness.verification.get_validator_status(&validator);
        if status == scoutchain_verification::ValidatorStatus::NotRegistered {
            return Err(format!("Validator {:?} not registered", validator));
        }
    }
    Ok(())
}

/// Invariant 4: No orphaned storage
/// Every player_id the harness registered still has a readable profile.
/// (Uses the public `get_player` API rather than poking private storage keys.)
pub fn assert_no_orphaned_storage(harness: &Harness) -> Result<(), String> {
    for &pid in &harness.player_ids {
        match harness.registration.try_get_player(&pid) {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                return Err(format!(
                    "Orphaned player_id in harness index: {pid} ({e:?})"
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Orphaned player_id in harness index: {pid} (host error {e:?})"
                ));
            }
        }
    }
    Ok(())
}

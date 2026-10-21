//! Chaos/concurrency testing framework for ScoutChain.
//!
//! Runs randomized schedules mixing operations across all four contracts
//! against a shared pool of players, scouts, and validators, then asserts
//! platform-wide invariants after each schedule.
//!
//! "Concurrent" here means randomized interleaving order of independent
//! transactions within the test harness, not literal OS-level parallelism —
//! Soroban's Env test harness runs single-threaded/deterministic by design.

mod fixtures;
mod invariants;
mod schedule;

use fixtures::Harness;
use invariants::{
    assert_fee_conservation, assert_level_monotonicity, assert_no_orphaned_storage,
    assert_validator_consistency,
};
use schedule::ScheduleGenerator;

/// Number of randomized schedules to run per CI invocation.
/// Bounded to keep CI time predictable — see ci/cpu-cost-budget.md for the
/// rationale behind this budget.
const SCHEDULES: u32 = 20;
/// Maximum operations per schedule.
const MAX_OPS: u32 = 50;

#[test]
fn chaos_random_schedules_preserve_global_invariants() {
    let mut failures = Vec::new();

    for schedule_idx in 0..SCHEDULES {
        let mut harness = Harness::setup();
        let mut generator = ScheduleGenerator::new(42 + schedule_idx as u64);

        let ops = generator.generate(MAX_OPS);
        let mut schedule_log = Vec::new();

        // Snapshot prior levels so assert_level_monotonicity can compare
        // against the pre-schedule state, not just the in-schedule history.
        harness.snapshot_levels();

        for (op_idx, op) in ops.iter().enumerate() {
            match harness.apply(op) {
                Ok(_) => {}
                Err(_) => {
                    // Some operations are expected to fail (e.g. cap limits,
                    // already-contacted, invalid CID path, cooldown).
                    // Record but continue.
                    schedule_log.push((op_idx, format!("{op:?}"), "failed"));
                }
            }
        }

        // Check global invariants after the schedule
        let invariant_checks: Vec<(&str, Result<(), String>)> = vec![
            ("fee_conservation", assert_fee_conservation(&harness)),
            ("level_monotonicity", assert_level_monotonicity(&harness)),
            (
                "validator_consistency",
                assert_validator_consistency(&harness),
            ),
            ("no_orphaned_storage", assert_no_orphaned_storage(&harness)),
        ];

        for (name, result) in invariant_checks {
            if let Err(e) = result {
                failures.push(format!(
                    "Schedule {schedule_idx}: invariant '{name}' failed: {e}\nSchedule: {:?}",
                    schedule_log
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Chaos test failures:\n{}",
        failures.join("\n\n")
    );
}

# CPU-instruction cost budgets

This file documents the CPU-instruction cost budgets enforced by
`tests/cost_budget.rs` in each contract package (`contracts/*/tests/cost_budget.rs`).
Those files are the source of truth — the numbers here mirror the checked-in
Rust constants so the current budget is visible without reading test code.

Each test:

1. Deploys and initializes the contract in a local `soroban_sdk::Env`.
2. Performs any setup calls needed to reach a realistic pre-state.
3. Calls `env.cost_estimate().budget().reset_default()` to reset cost
   tracking and switch to the default (mainnet-like) resource-limit model.
4. Invokes the representative operation being measured.
5. Reads `env.cost_estimate().budget().cpu_instruction_cost()` and asserts
   it is within the budget below, failing with a message naming the
   operation, the measured cost, and the overage (in instructions and %)
   when it is not.

These tests run as part of `cargo test --workspace` (the existing CI `test`
job already runs this) and their `--nocapture` output is additionally
captured and uploaded as the `cpu-cost-budget-<sha>` CI artifact so
measured-cost trends can be tracked across commits.

## Current budgets

| Contract       | Operation                       | Budget (CPU instructions) |
|----------------|----------------------------------|---------------------------|
| registration   | `register_player`                | 20,000,000                |
| registration   | `update_profile`                 | 10,000,000                |
| registration   | `filter_players`                 | 15,000,000                |
| verification   | `register_validator`             | 15,000,000                |
| verification   | `approve_milestone`              | 20,000,000                |
| verification   | `get_validator_milestones_page`  | 15,000,000                |
| verification   | `revoke_validator(ForCause, limit=50, total=500)` | 50,000,000 |
| progress       | `advance_level`                  | 15,000,000                |
| progress       | `reset_player_level`             | 12,000,000                |
| progress       | `get_progress_history_page`      | 10,000,000                |
| progress       | `verify_history_proof`           | 8,000,000                 |
| scout_access   | `subscribe`                      | 20,000,000                |
| scout_access   | `pay_to_contact`                 | 20,000,000                |
| scout_access   | `batch_contact_players` (5 ids)  | 25,000,000                |
| scout_access   | `expire_trial_offers` (limit=20) | 25,000,000                |
| scout_access   | `get_expiring_subscriptions` (20 scouts, buckets ~50k days from epoch, limit 50) | 1,500,000 |

All budgets above were calibrated from real `cargo test --test cost_budget
-- --nocapture` measurements (see `cpu-cost-budget-report.txt`) with 20%
headroom via `scripts/calibrate-budgets.py`. The Merkle history commitment
recomputation added by issue #700 is included in the `advance_level` and
`reset_player_level` measurements above — its cost is well within the
calibrated budgets.

These budgets are calibrated automatically by `scripts/calibrate-budgets.py`
from the `cpu-cost-budget-report.txt` CI artifact.  The script adds a
documented headroom percentage (default 20%) to the latest measured cost.
To re-calibrate manually:
  1. Ensure `cpu-cost-budget-report.txt` is present locally (produced by
     `cargo test --workspace --test cost_budget -- --nocapture`).
  2. Run: `CALIBRATE_WRITE=1 python scripts/calibrate-budgets.py`

## Raising a budget

Legitimate feature growth (a new storage write, an added validation pass,
etc.) can push an operation's cost above its budget. When that happens:

1. Bump the relevant `*_CPU_BUDGET` constant in the corresponding
   `contracts/<name>/tests/cost_budget.rs`.
2. Update the matching row in the table above to the same value.
3. In the PR description, add a one-line justification explaining why the
   growth is expected and acceptable (e.g. "adds a second persistent write
   for the new X index, +Y instructions").

Budgets are per-operation and independent of the WASM binary size budget in
`ci/wasm-size-budget.json` (see that file's own raising process, which
follows the same pattern) — a contract can grow in size without any single
operation's instruction cost regressing, and vice versa.

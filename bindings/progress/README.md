# ScoutChain Progress Bindings

TypeScript client bindings for the ScoutChain progress contract. For setup, generation, and usage instructions, see the [bindings README](../README.md); for available contract functions, see the [contract reference](../../docs/CONTRACT_REFERENCE.md).

For a reference implementation that reads a player's current level and full
advancement history (including paginated and incremental variants), see
[`examples/getPlayerHistory.ts`](examples/README.md).

<!-- AUTO-GENERATED FUNCTION LIST BEGIN - DO NOT EDIT MANUALLY -->

## Functions

The following functions are available in this contract. For complete documentation including parameters, return types, authorization requirements, and examples, see [CONTRACT_REFERENCE.md](../../docs/CONTRACT_REFERENCE.md#progress).

- `initialize(admin: Address) -> Result<(), ProgressError>` — One-time contract setup.
- `propose_admin(new_admin: Address) -> Result<(), ProgressError>` — Store or replace a pending admin proposal. The current admin retains all privileges until the proposed address accepts.
- `accept_admin() -> Result<(), ProgressError>` — Finalize the transfer. The stored pending admin must sign, proving control of the address. Acceptance updates the admin and clears the proposal.
- `transfer_admin(new_admin: Address) -> Result<(), ProgressError>` — Deprecated compatibility alias for `propose_admin`. It creates or replaces a proposal and does not immediately change the admin.
- `reset_player_level(player_id: u64, target_level: ProgressLevel) -> Result<(), ProgressError>` — Reset a player's progress level for dispute resolution or correction. Existing history is preserved; a new `ProgressEntry` recording the reset is appended. `milestone_ref` is `0` for admin resets.
- `advance_level(caller: Address, player_id: u64, milestone_ref: u32) -> Result<ProgressLevel, ProgressError>` — Advance a player's progress level by one tier. `milestone_ref` links back to the verification contract's milestone index. Returns the new `ProgressLevel`.
- `get_level(player_id: u64) -> ProgressLevel` — Return the player's current progress level. Returns `Unverified` for unknown player IDs (no `PlayerNotFound` error).
- `get_history_count(player_id: u64) -> u32` — Return the total number of history entries recorded for a player.
- `get_history_entry(player_id: u64, index: u32) -> Result<ProgressEntry, ProgressError>` — Read a specific history entry. Indices start at `1`. Each `ProgressEntry` includes `updated_at` in Unix seconds and `ledger_sequence: u32`, the Soroban ledger sequence number at the time of the change (not a timestamp), for tamper-proof auditability.
- `get_progress_history(player_id: u64) -> Vec<ProgressEntry>` — Return all history entries for a player in chronological order. Internally reads a single `HistoryVec` persistent storage key regardless of entry count — O(1) reads instead of the previous O(N) loop. Returns an empty `Vec` for unknown player IDs.
- `pause_contract() -> Result<(), ProgressError>` — Halt all state-changing operations.
- `unpause_contract() -> Result<(), ProgressError>` — Resume normal operations after a pause.
- `health() -> ContractHealth` — Return the contract's initialization and pause status.
- `set_verification_contract(addr: Address) -> Result<(), ProgressError>` — Store the verification contract address so `advance_level` can authenticate cross-contract callers. Without this, only direct `caller` auth is accepted (useful for testing). Admin only.
- `set_registration_contract(addr: Address) -> Result<(), ProgressError>` — Store the registration contract address so `advance_level` can sync player levels via cross-contract call. Admin only.
- `set_scout_access_contract(addr: Address) -> Result<(), ProgressError>` — Whitelist the scout_access contract as a secondary authorized caller of `advance_level` (for trial-offer Level-3 advances). Admin only.
- `upgrade(new_wasm_hash: BytesN<32>) -> Result<(), ProgressError>` — Replace the contract WASM in-place. Persistent storage (admin, history) survives the upgrade. Admin only.
- `get_progress_history_page(player_id: u64, offset: u32, limit: u32) -> Vec<ProgressEntry>` — Paginated history retrieval. Returns entries starting at `offset+1`. `limit` is clamped to the range 1 through 50. Returns an empty `Vec` when `offset` >= total count.
- `get_history_since(player_id: u64, since_timestamp: u64) -> Vec<ProgressEntry>` — Return all of a player's history entries with `updated_at >= since_timestamp` (Unix seconds). Useful for indexers polling for changes since their last sync point instead of re-reading the full history.
- `version() -> String` — Return the deployed contract version string (from `Cargo.toml` at build time).

<!-- AUTO-GENERATED FUNCTION LIST END -->

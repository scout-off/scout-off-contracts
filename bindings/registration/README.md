# ScoutChain Registration Bindings

TypeScript client bindings for the ScoutChain registration contract. For setup, generation, and usage instructions, see the [bindings README](../README.md); for available contract functions, see the [contract reference](../../docs/CONTRACT_REFERENCE.md).

For a reference implementation of a real `update_profile` submission (built and
verified against this client, not a stub transaction ID), see
[`examples/updateProfile.ts`](examples/README.md).

<!-- AUTO-GENERATED FUNCTION LIST BEGIN - DO NOT EDIT MANUALLY -->

## Functions

The following functions are available in this contract. For complete documentation including parameters, return types, authorization requirements, and examples, see [CONTRACT_REFERENCE.md](../../docs/CONTRACT_REFERENCE.md#registration).

- `initialize(admin: Address) -> Result<(), ScoutChainError>` — One-time contract setup. Must be called before any other function.
- `propose_admin(new_admin: Address) -> Result<(), ScoutChainError>` — Store or replace a pending admin proposal. The current admin retains all privileges until the proposed address accepts.
- `accept_admin() -> Result<(), ScoutChainError>` — Finalize the pending transfer. The stored pending admin must sign, proving control of the address. Acceptance updates the admin and clears the proposal.
- `transfer_admin(new_admin: Address) -> Result<(), ScoutChainError>` — Deprecated compatibility alias for `propose_admin`. It does not immediately change the admin; the proposed address must still call `accept_admin`.
- `register_player(wallet: Address, vitals: PlayerVitals, ipfs_hashes: Vec<String>) -> Result<u64, ScoutChainError>` — Create a new on-chain player profile at Level 0 (Unverified). Returns the assigned `player_id`.
- `update_profile(player_id: u64, ipfs_hashes: Vec<String>) -> Result<(), ScoutChainError>` — Replace a player's IPFS content hashes (highlight reels, photos). Note that `update_profile` accepts only `ipfs_hashes` and does not take or modify `PlayerVitals` fields. Because player vitals are write-once at registration time and immutable post-registration, length validation runs exclusively during `register_player` and no post-registration update path exists to set or modify vitals.
- `deregister_player(player_id: u64) -> Result<(), ScoutChainError>` — Remove a player profile and all associated wallet index entries. Implements the GDPR right-to-erasure. The `player_id` is permanently freed.
- `deactivate_player(player_id: u64) -> Result<(), ScoutChainError>` — Hide a player from `filter_players` results without erasing their profile (soft-delete). Sets the `PlayerDeactivated` flag; the player's data and `player_id` remain intact and can be restored with `reactivate_player`. Emits a `player_deactivated` event on success.
- `reactivate_player(player_id: u64) -> Result<(), ScoutChainError>` — Reverse a prior `deactivate_player` call. Clears the `PlayerDeactivated` flag, making the player visible in `filter_players` results again. Emits a `player_reactivated` event on success.
- `register_scout(wallet: Address, region: String) -> Result<u64, ScoutChainError>` — Create a new scout profile. Returns the assigned `scout_id`. Scouts start as unverified (`verified: false`); call `verify_scout` to promote.
- `verify_scout(scout_id: u64) -> Result<(), ScoutChainError>` — Mark a scout as verified. Verified scouts gain trust-signal visibility on the discovery dashboard.
- `set_progress_contract(addr: Address) -> Result<(), ScoutChainError>` — Store the progress contract address so `set_player_level` may only be called by that contract. Must be called after both contracts are deployed (admin only).
- `set_player_level(player_id: u64, level: ProgressLevel) -> Result<(), ScoutChainError>` — Update a player's stored `ProgressLevel`. Only callable by the registered progress contract address via cross-contract invocation.
- `get_player(player_id: u64) -> Result<PlayerProfile, ScoutChainError>` — Retrieve the full player profile including wallet, vitals, IPFS hashes, and current progress level.
- `get_player_by_wallet(wallet: Address) -> Result<PlayerProfile, ScoutChainError>` — Look up a player profile by their Stellar wallet address. Useful when the `player_id` is unknown.
- `get_scout(scout_id: u64) -> Result<ScoutProfile, ScoutChainError>` — Retrieve a scout profile by ID.
- `get_player_count() -> u64` — Return the total number of registered players. Returns `0` before the contract is initialized.
- `get_scout_count() -> u64` — Return the total number of registered scouts. Returns `0` before the contract is initialized.
- `filter_players(region: String, position: String, min_level: ProgressLevel) -> Result<Vec<PlayerProfile>, ScoutChainError>` — Scout discovery query. Returns up to 50 player profiles matching the given region, position, and minimum progress level.
- `pause_contract() -> Result<(), ScoutChainError>` — Halt all state-changing operations (circuit breaker). Read-only queries remain available.
- `unpause_contract() -> Result<(), ScoutChainError>` — Resume normal operations after a pause.
- `health() -> ContractHealth` — Return the contract's initialization and pause status.
- `get_player_summary(player_id: u64) -> Result<PlayerSummary, ScoutChainError>` — Return a lightweight player view without IPFS hashes or wallet address. Useful for scout discovery lists where the full profile is not needed.
- `get_players(ids: Vec<u64>) -> Result<Vec<PlayerSummary>, ScoutChainError>` — Batch-fetch lightweight player summaries for up to 20 IDs in a single call. Missing IDs are silently skipped (partial hits are returned without error). For cost rationale behind batch-size caps, see the batch-operation entries in [`ci/cpu-cost-budget.md`](../ci/cpu-cost-budget.md), including `scout_access.batch_contact_players`.
- `get_scouts(ids: Vec<u64>) -> Result<Vec<ScoutProfile>, ScoutChainError>` — Batch-fetch full scout profiles for up to 20 IDs in a single call. Mirrors `get_players` semantics exactly: missing IDs are silently skipped with partial hits returned successfully, and the same 20-ID cap applies.
- `version() -> String` — Return the deployed contract version string (from `Cargo.toml` at build time).

<!-- AUTO-GENERATED FUNCTION LIST END -->

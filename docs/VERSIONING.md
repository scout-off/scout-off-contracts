# Contract Versioning Policy

## Semantic Versioning

ScoutChain contracts follow [Semantic Versioning 2.0.0](https://semver.org/) — `MAJOR.MINOR.PATCH`:

| Component | Incremented when |
|-----------|-----------------|
| **MAJOR** | Breaking change — storage layout changed, function removed, error codes renumbered, event schema changed |
| **MINOR** | Backward-compatible addition — new function, new event, new error code appended at end of enum |
| **PATCH** | Backward-compatible fix — bug fix, gas optimisation, documentation update in source |

The current workspace version of all four contracts is **v1.1.0**. Contract-specific
releases in the Version History table may describe changes that were deployed to
only one contract, but the shared workspace version remains the build-time value
reported by every contract's `version()` function.

> **Note:** `Cargo.toml` `[workspace.package].version` is the build-time source of truth; keep the Version History table below in sync with every Cargo version bump.

> **Bindings packages:** the TypeScript binding packages under `bindings/` are
> versioned in lockstep with the workspace version.
> `scripts/generate-bindings.sh` derives the version from `Cargo.toml` and
> rewrites each generated `package.json` after generation (the CLI overwrites
> the scaffold with its own placeholder version), so a workspace version bump
> automatically propagates to the bindings on the next regeneration.

Each contract exposes a `version()` function that returns its current version string:

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID  -- version
stellar contract invoke --id $VERIFICATION_CONTRACT_ID  -- version
stellar contract invoke --id $PROGRESS_CONTRACT_ID      -- version
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID  -- version
```

---

## What Constitutes a Breaking Change

A change is **breaking** (requires a MAJOR bump) if any of the following are true:

- A `pub fn` is renamed or removed from any contract
- A function's parameter list changes (added, removed, or reordered parameters)
- A `#[contracterror]` variant is renumbered or removed
- A `#[contracttype]` struct or enum used in storage or function signatures gains or loses a field
- The storage key layout changes such that existing persistent-storage entries become unreadable
- An on-chain event's topic or data schema changes in a backward-incompatible way
- The cross-contract interface expected by `set_progress_contract` / `set_verification_contract` changes

A change is **non-breaking** (MINOR or PATCH) if:

- A new `pub fn` is added (existing callers are unaffected)
- A new `#[contracterror]` variant is appended at the end of an enum (existing numeric codes unchanged)
- A new event type is added (existing listeners ignore unknown topics)
- Internal helper functions or private storage keys change

---

## Upgrade Checklist

The upgrade procedure is implemented in `scripts/upgrade.sh` (see [DEPLOYMENT.md — Upgrading a Deployed Contract](DEPLOYMENT.md#upgrading-a-deployed-contract) for manual steps).

```bash
./scripts/upgrade.sh <network> <contract_name> <new_wasm_path>
# Example:
./scripts/upgrade.sh testnet verification target/wasm32v1-none/release/scoutchain_verification.wasm
```

### Pre-upgrade

- [ ] Read all BREAKING CHANGES listed in the release notes for the target version
- [ ] Snapshot current on-chain state that lives in **instance** storage (fee config, initialized flag, contract links) — these survive the WASM swap but must be re-verified
- [ ] Check `version()` on all four contracts to confirm the baseline version before upgrade. Each contract should return the expected deployed workspace version (currently `1.1.0`, from `CARGO_PKG_VERSION`, with no `v` prefix).
- [ ] Run `cargo test --workspace` against the new code locally
- [ ] Rehearse the upgrade locally with the storage-survival harness — **no testnet fees required.** For each contract it deploys v1, seeds representative state, calls `upgrade()`, and asserts every row of the "What survives an upgrade" table in `docs/DEPLOYMENT.md` (persistent state unchanged; instance `Initialized`/`Paused` flags intact; cross-contract links re-wirable), including the `verification` `AlreadyConfigured` re-wire quirk. Run:
  - `cargo test -p scoutchain-registration  --test upgrade_rehearsal`
  - `cargo test -p scoutchain-verification  --test upgrade_rehearsal`
  - `cargo test -p scoutchain-progress      --test upgrade_rehearsal`
  - `cargo test -p scoutchain-scout-access  --test upgrade_rehearsal`
  - (or all at once: `cargo test --workspace --test upgrade_rehearsal`)
- [ ] Test the full upgrade flow on testnet before touching mainnet (only after the local rehearsal above passes, so testnet transaction fees are spent on a flow you already know survives an upgrade)

### During upgrade (per contract)

- [ ] Build and install the new WASM: `stellar contract build && stellar contract install ...`
- [ ] Call `upgrade(new_wasm_hash)` from the admin address
- [ ] Immediately call `health()` to confirm the contract responds

### Post-upgrade

- [ ] Call `version()` on each upgraded contract to confirm the expected new version
- [ ] Re-verify instance storage (fee config, contract links) — re-apply if values were wiped
- [ ] Verify cross-contract wiring: `./scripts/verify-cross-contract-wiring.sh <network>`
- [ ] Re-run cross-contract wiring if any contract was re-deployed from scratch: `./scripts/initialize.sh <network>`
- [ ] Regenerate TypeScript bindings: `./scripts/generate-bindings.sh <network>`
- [ ] Update backend and frontend repos with the new bindings

---

## v0.1.0 → v0.x.0 Migration Notes

This is the initial release. No prior on-chain state exists. The migration path from v0.1.0 to any future v0.x.0 (minor, backward-compatible) release is:

1. **Build the new WASM** for the changed contract(s).
2. **Install and upgrade** each changed contract using the procedure in [DEPLOYMENT.md](DEPLOYMENT.md#upgrading-a-deployed-contract).
3. **Re-verify instance storage** — fee config and contract links are in instance storage and must be confirmed after each WASM swap.
4. **Re-wire cross-contract links** if any contract address changed (i.e., a contract was re-deployed rather than upgraded in-place).
5. **Regenerate bindings** and redeploy the backend/frontend.

### Storage compatibility (v0.1.0 baseline)

All persistent-storage keys in v0.1.0 use the `DataKey` enum defined in each contract's `types.rs`. Any v0.x.0 release that adds new `DataKey` variants is backward-compatible. Any release that **renames or removes** a `DataKey` variant is a breaking change and requires a MAJOR bump plus a data migration script.

### Error code compatibility (v0.1.0 baseline)

Error code assignments for v0.1.0 are fixed as documented in [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md). Future minor releases may only **append** new error codes at the end of each enum. SDK consumers should handle unknown error codes gracefully (treat them as unexpected errors and surface to the user).

> **Known gap:** `ScoutAccessError` code 13 is intentionally reserved and will never be assigned. See `contracts/scout_access/src/errors.rs` for the inline explanation.

---

## Version History

### Format & Entry Guidelines

When adding new entries to the Version History table:
- **Contract Scope**: All four contracts (`registration`, `verification`, `progress`, `scout_access`) were initially released together at `v0.1.0`. Future releases may update all contracts in lockstep or target specific contracts individually. Specify the scope in the **Version** column (e.g., `v0.2.0 (all)` or `v0.2.0 (verification)`).
- **SemVer Bump Type**: Explicitly classify each change as `MAJOR` (breaking storage/API change), `MINOR` (backward-compatible feature/event/error addition), or `PATCH` (backward-compatible bug fix/gas optimization) in the **Type** column.
- **Summary**: Provide a concise summary of changes, explicitly calling out breaking changes if `MAJOR`.
- **Cross-reference**: Every entry must mirror the corresponding entry in [CHANGELOG.md](../CHANGELOG.md) — keep both files in sync.

> **Enforced by CI:** The `abi-diff` job fails any MAJOR/MINOR ABI change unless
> it also adds a matching Version History row in this table alongside the
> corresponding `CHANGELOG.md` entry.

| Version | Date | Type | Summary |
|---------|------|------|---------|
| v0.1.0 (all) | 2025 | MINOR | Initial release — all four contracts with full test coverage |
| v0.2.0 (scout_access) | 2026-07-28 | MAJOR | BREAKING: `ContactQuotaExceeded` (18) deprecated; `batch_contact_players` now returns `ProContactLimitReached` (20) for Pro-tier quota exceeded; error code 18 slot reserved |
| v0.2.0 (verification) | 2026-07-29 | MINOR | Added `attest_milestone` k-of-n threshold consensus for milestone approval (new fns, 3 error codes appended: 26-28, retroactive vote invalidation on validator revocation); `approve_milestone` unchanged by default (`threshold = 1`) |
| v0.3.0 (scout_access) | 2026-08-18 | MINOR | Added escrow-backed trial offers: `log_trial_offer` now charges `trial_offer_escrow_stroops`, `expire_trial_offers(limit)` sweeps stale entries after `trial_offer_expiry_secs`, and `admin_refund_trial_escrow` provides a targeted recovery path for individual stuck escrows. |
| v0.3.0 (all) | 2026-08-18 | MINOR | Completed cross-contract wiring observability rollout (issue #1041): `get_wiring_state()` on all four contracts, per-link re-wiring epoch + `wiring_updated` event on every setter, verification's legacy first-call-only guards preserved unchanged. All new storage keys additive — see CHANGELOG.md for the full summary |
| v0.3.1 (scout_access) | 2026-08-18 | MINOR | Implemented `EvidenceAccessGrant` confidential-evidence access tracking: `pay_to_contact` and `batch_contact_players` record a grant, `has_evidence_access` / `get_evidence_access_grant` / `get_player_access_grants` expose it, and `admin_revoke_evidence_access` makes a grant non-active without deleting the historical record. |
| v0.4.0 (verification) | 2026-08-19 | MAJOR | BREAKING: Added dispute-jury escalation for high-impact milestone disputes. `MilestoneDispute` grows with `impact_score`, `jury_required`, `quorum`, `votes_for`, `votes_against`, and `voting_deadline`; `set_jury_config`, `cast_dispute_vote`, and `tally_dispute` add the full jury flow, while low-impact disputes remain admin-resolved. Requires migration for existing stored disputes. |
| v1.0.0 (verification) | 2026-08-19 | MAJOR | BREAKING: `revoke_validator` and `batch_revoke_validators` parameter lists changed — explicit `RevocationSeverity` enum replaces magic-string severity inference. Added for-cause cascade sweep (`run_cascade_sweep`, `continue_revocation_cascade`), `is_milestone_flagged`, `rereview_milestone`, `get_revocation_record`. New error codes 32 (`NotEligibleToReReview`) and 33 (`MilestoneNotFlagged`). New events: `milestone_flagged`, `milestone_flag_cleared`, `revocation_cascade_complete`, `revocation_cascade_continued`. See CHANGELOG.md and docs/VALIDATOR_REVOCATION_REREVIEW.md for full details. |
| v1.1.0 (all) | 2026-08-20 | MINOR | Added unauthenticated scalar peer-address getters (issue #1116) for six of the platform's eight cross-contract wiring links (see `docs/WIRING_REGISTRY_DESIGN.md` for the full list); each returns `None` until configured and leaves the aggregate wiring-state APIs unchanged. |
<!-- Template / Example for future entries: -->
<!-- | v0.2.0 (verification) | YYYY-MM-DD | MINOR | Added batch verification helper functions | -->
<!-- | v1.0.0 (all) | YYYY-MM-DD | MAJOR | BREAKING: Updated storage key layout across all contracts | -->

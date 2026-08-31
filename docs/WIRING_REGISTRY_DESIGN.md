# Cross-Contract Wiring Registry — Design Document

**Issue**: #801, completed by #1041  
**Status**: Complete — `get_wiring_state()` on all four contracts, epoch-based
partial-rewiring detection, and a rewritten verification script.

---

## Problem Statement (original, #801)

ScoutChain's four contracts are interconnected by peer-address pointer
fields. The table below enumerates all **eight** links. It originates from
the #801 design doc — which listed only six — and has been corrected to
match the current code and the **"The Full Picture"** table below; the two
tables now agree. "The Full Picture" remains the authoritative reference and
adds the per-link owner/target and epoch details.

| Contract | Setter | Storage Key | Re-wiring guard |
|----------|--------|-------------|-----------------|
| `verification` | `set_progress_contract` | `ProgressContract` | First-call-only (`AlreadyConfigured`); use `update_progress_contract` after |
| `verification` | `set_registration_contract` | `RegistrationContract` | First-call-only (`AlreadyConfigured`); use `update_registration_contract` after |
| `registration` | `set_progress_contract` | `ProgressContract` | None — freely re-settable |
| `progress` | `set_verification_contract` | `VerificationContract` | None |
| `progress` | `set_registration_contract` | `RegistrationContract` | None |
| `progress` | `set_scout_access_contract` | `ScoutAccessContract` | None |
| `scout_access` | `set_progress_contract` | `ProgressContract` | None |
| `scout_access` | `set_registration_contract` | `RegistrationContract` | None |

Today there is **no on-chain mechanism** to ask "are all links mutually
consistent?" — `scripts/verify-cross-contract-wiring.sh` polls each contract
externally, but it can only confirm the contracts are alive; it cannot yet read
the stored peer addresses (no public getter functions exist for them).

The asymmetric re-wiring guards (first-call-only vs freely re-settable) are
themselves a documented source of confusion in `docs/DEPLOYMENT.md`.

---

## Approach Comparison

### Option A — Shared-types helper only (no new contract)

Each contract exposes a `get_wiring_state()` public getter that returns a
struct listing all peer addresses it holds. A new `check_wiring_consistency()`
function in `shared-types` can be called by any off-chain caller or CI script
to compute a consistency snapshot without any on-chain coordination.

**Pros:**
- Zero extra cross-contract hops at runtime — no gas cost increase.
- No new contract to deploy or upgrade.
- Fully backward-compatible with already-deployed testnet contracts (add getters
  in the next upgrade, old logic unchanged).
- Simple mental model: "call `get_wiring_state()` on each contract and compare."

**Cons:**
- The consistency check is still an off-chain step — contracts cannot enforce
  wiring correctness within a transaction.
- Four separate `get_wiring_state()` calls are needed for a full picture.

### Option B — Dedicated registry contract

A fifth `WiringRegistry` contract holds all eight peer addresses as the single
source of truth. All four contracts query the registry at runtime via
cross-contract call to resolve peer addresses instead of reading local storage.

**Pros:**
- On-chain consistency — updating one entry in the registry is instantly
  visible to all four contracts on the next call.
- Admin workflow is centralised in one contract.

**Cons:**
- **+1 cross-contract hop** on every `approve_milestone`, `subscribe`,
  `batch_contact_players`, and `advance_level` call — at minimum one extra
  `invoke_contract` per transaction, which adds ~500–2,000 CPU instructions
  and a small ledger-read fee per call.
- Requires deploying and initializing a fifth contract.
- Upgrade complexity increases: all four contracts must be upgraded to use the
  new registry before any can be cut over. A partial upgrade leaves a split
  environment.
- **Not backward-compatible** with already-deployed testnet contracts without a
  coordinated multi-contract migration.

### Recommendation: Option A (shared-types getter approach)

The registry-contract approach's runtime overhead and migration complexity
outweigh its benefits for a system where re-wiring happens once at deployment
(rarely more than a few times over a contract's lifetime). Option A achieves
the same operational visibility at zero runtime cost and is safe to roll out
incrementally across upgrades.

---

## Prototype: `get_wiring_state()` on the Progress Contract

The progress contract holds the most wiring links (three: registration,
verification, scout_access), making it the highest-value starting point.

### Storage schema (existing)

```
DataKey::RegistrationContract  → Address
DataKey::VerificationContract  → Address
DataKey::ScoutAccessContract   → Address
```

### New public getter (prototype — implemented in this PR)

```rust
pub fn get_wiring_state(env: Env) -> ProgressWiringState
```

Returns a `ProgressWiringState` struct:

```rust
#[contracttype]
pub struct ProgressWiringState {
    /// Address of the registration contract, if set.
    pub registration_contract: Option<Address>,
    /// Address of the verification contract, if set.
    pub verification_contract: Option<Address>,
    /// Address of the scout_access contract, if set.
    pub scout_access_contract: Option<Address>,
}
```

### Consistency check helper

`ProgressWiringState::is_fully_wired()` returns `true` iff all three addresses
are `Some(_)`. The external verification script uses this to report incomplete
wiring without needing to enumerate storage keys manually.

---

## The Full Picture (issue #1041)

Eight peer-address pointers exist across the four contracts — not five.
`verification` holds two (`ProgressContract`, `RegistrationContract` — the
latter added by #1014, after the original version of this doc was written);
`progress` holds three; `registration` holds one; `scout_access` holds two.

| # | Owner | Setter | Target | Re-wiring guard |
|---|-------|--------|--------|-----------------|
| 1 | `verification` | `set_progress_contract` / `update_progress_contract` | `progress` | First-call-only, deprecated-but-functional |
| 2 | `verification` | `set_registration_contract` / `update_registration_contract` | `registration` | First-call-only, deprecated-but-functional |
| 3 | `registration` | `set_progress_contract` | `progress` | None — freely re-settable |
| 4 | `progress` | `set_verification_contract` | `verification` | None |
| 5 | `progress` | `set_registration_contract` | `registration` | None |
| 6 | `progress` | `set_scout_access_contract` | `scout_access` | None |
| 7 | `scout_access` | `set_progress_contract` / `update_progress_contract` (alias) | `progress` | None |
| 8 | `scout_access` | `set_registration_contract` | `registration` | None |

Every setter above now bumps a per-link `epoch` and emits `wiring_updated` on
every successful call, in addition to whatever legacy event it already
emitted (`progress_contract_updated`, `registration_contract_updated`), and
every contract now implements `get_wiring_state()`.

### Re-wiring policy: majority freely-re-settable, two deliberate exceptions

Links #1 and #2 (both on `verification`) keep their pre-existing
first-call-only guard (`AlreadyConfigured`, with `update_progress_contract` /
`update_registration_contract` as the escape hatch) rather than being
homogenised to match the other six. This is intentional, not an oversight:

- The constraint for this rollout is explicit — "must not remove or break
  verification's existing `AlreadyConfigured`/`update_progress_contract`
  behavior for any already-deployed contract relying on it." Any already-live
  `verification` deployment that has scripted around this guard (e.g. always
  calling `update_progress_contract` on redeploy, never `set_progress_contract`)
  must keep working unmodified.
- Removing the guard would also be a **storage-semantics-neutral but
  behavior-breaking** change: `DataKey::ProgressContractSet` /
  `DataKey::RegistrationContractSet` stay in storage either way, so
  `scripts/check-storage-layout-compat.sh` would not catch a silent removal —
  only integration tests would. The two regression tests added for this
  rollout (`test_set_progress_contract_second_call_returns_already_configured`,
  `test_update_progress_contract_succeeds`, and their `registration_contract`
  counterparts) exist specifically to keep this guard from silently regressing
  in either direction in future upgrades.

Every other link was already freely re-settable before this rollout (the
"majority precedent" the original design doc anticipated in its Migration
Note) and stays that way — this rollout only adds epoch bookkeeping and the
`wiring_updated` event to them, not a new guard.

### Epoch-based partial-rewiring detection

Every `WiringLink { address: Option<Address>, epoch: u32 }`
(`scoutchain_shared_types::WiringLink`, shared by all four contracts) pairs
the peer address with a monotonically-incrementing epoch, bumped by
`write_wiring_link` on every successful set/update call.

**Why epoch, when `Option<Address>` already distinguishes "never configured"
from "configured to something"?** Because the interesting failure mode this
issue exists to catch — one contract's peer pointer updated, another's left
stale mid-migration — is not actually about a *single* link's history; it's
about **comparing several independent links that should all agree**. Three of
the eight links above name `progress` as their target (#1, #3, #7); three
name `registration` (#2, #5, #8); `verification` and `scout_access` are each
named by exactly one. `scripts/verify-cross-contract-wiring.sh` groups links
by target contract and classifies each group:

- **`FULLY_WIRED`** — every link naming this target has the target's actual
  deployed address.
- **`NEVER_CONFIGURED`** — every link naming this target is unset (`epoch ==
  0`) — a fresh deployment that hasn't been wired yet.
- **`PARTIAL`** — anything else. This is the case this issue exists to catch:
  an operator redeploys `progress` and updates `verification.ProgressContract`
  and `registration.ProgressContract` before a script crash or auth failure
  interrupts the third call to `scout_access.set_progress_contract` — two
  members of the `progress` group now correctly point at the new deployment,
  one still points at the old one (or was never set). Distinguishing this
  from `NEVER_CONFIGURED` matters operationally: a fresh unwired deployment
  needs the normal `initialize.sh` wiring sequence; a `PARTIAL` group needs a
  single targeted corrective call, which `--repair` prints exactly.

`epoch`'s role in this: `address`'s `None`/`Some` already tells the script
whether a *single* link is configured, so epoch isn't needed for the
per-link classification itself. What it adds is operator-facing diagnosis
*after* a repair attempt — comparing the epoch this script reports on a
second run against the first tells an operator whether their corrective
`stellar contract invoke` call actually landed (epoch increased) or silently
didn't (epoch unchanged — an auth or network problem, not a wrong-address
problem). See the extensive rationale comment at the top of
`scripts/verify-cross-contract-wiring.sh` for the full write-up, including
why this script deliberately does not persist an epoch baseline between runs.

### Why detect-and-guide, not auto-repair

Soroban has no atomic multi-contract transaction primitive available to these
admin scripts — every `stellar contract invoke` is independently failable.
Auto-correcting a detected `PARTIAL` group by having the script itself invoke
the missing `set_*_contract` calls would mean the *tool* now makes
admin-controlled state changes, and if its own corrective calls partially
fail there is no longer any way to distinguish "the original interrupted
re-wiring" from "this script's own interrupted repair attempt." `--repair`
prints the exact corrective command(s) for a human to run and re-verify; it
never invokes anything itself.

---

## Migration Note

### For already-deployed testnet contracts

1. **No immediate action required for functional correctness.** Every
   existing setter's behavior (including verification's two first-call-only
   guards) is unchanged for callers that don't care about `get_wiring_state()`
   or `wiring_updated`. The new epoch storage keys are purely additive.

2. **Upgrade all four contracts to gain the observability rollout.** Unlike
   the original single-contract (`progress`-only) rollout, this one only
   provides its full "detect a partial re-wiring" value once at least the two
   contracts on either side of a given link have both been upgraded — a
   not-yet-upgraded contract's links are reported as `UNAVAILABLE` (not
   `UNCONFIGURED`) and excluded from that target's group classification, so a
   partially-upgraded fleet degrades to "fewer links checked," never to a
   false `PARTIAL`/`FULLY_WIRED` verdict built on stale assumptions.

3. **Re-run `scripts/verify-cross-contract-wiring.sh` after upgrading** to
   confirm the newly-exposed links agree with what was already wired.
   `initialize.sh` now gates its own success on this same check for fresh
   deployments.

### Timeline

| Step | PR / Upgrade | Scope | Status |
|------|-------------|-------|--------|
| 1 | feat/797-798-801-835 | Add `get_wiring_state()` to progress; update verification script | ✅ Done |
| 2 | feat/1041-wiring-epoch-rollout | Add `get_wiring_state()` to registration, verification, scout_access, sharing a common `WiringLink` pattern from `shared-types` | ✅ Done |
| 3 | feat/1041-wiring-epoch-rollout | Homogenise re-wiring guards (epoch + `wiring_updated` on every setter) across all four contracts, with verification's two legacy guards kept deprecated-but-functional; epoch-based partial-rewiring detection in the verification script and `full-readiness-check.sh`; post-wiring gate in `initialize.sh` | ✅ Done |

---

## Open Questions — resolved

1. ~~Should `get_wiring_state()` be added to `shared-types` as a trait so the
   pattern is enforced at compile time?~~ **Resolved**: a trait requiring each
   contract's wiring-state struct to expose the same shape doesn't fit well
   here, since the four contracts hold different *numbers* of links (one,
   two, two, three) — a trait method returning "the wiring state" would still
   need a per-contract associated type, buying little over what a shared
   value type already gives. Instead, `shared-types` exports the `WiringLink`
   struct (`{ address: Option<Address>, epoch: u32 }`) plus
   `read_wiring_link` / `write_wiring_link` helpers that every contract's
   getter/setters call — this is what actually prevents the four
   implementations from silently drifting (identical storage semantics,
   identical epoch-bump logic), without forcing a shared trait over
   differently-shaped structs. `progress`'s pre-existing `ProgressWiringState`
   keeps its original three flat `Option<Address>` fields for backward
   compatibility (changing their type would be a breaking storage-layout
   change) with three new flat epoch fields appended; the other three
   contracts' wiring-state structs (new types, no backward-compat
   constraint) use `WiringLink` fields directly.

2. Should the progress contract's `advance_level` verify that its caller's
   address matches the stored verification/scout_access contract address? It
   already does this via `require_auth()`. The wiring state is therefore already
   enforced at call time — the registry approach would not add additional
   security here. (Unchanged from the original doc — still out of scope; see
   the "No new cross-contract runtime hops" constraint on issue #1041.)

# Sybil Resistance for Pro-Tier Pro Contact Limit (#808)

## Overview

This document addresses the economic incentive for scouts to bypass the Pro-tier quota by registering multiple wallets. At current pricing (Pro = 0.3 XLM/month for 10 contacts, Elite = 0.7 XLM/month for unlimited), a rational actor needs only 3 Pro wallets to equal Elite's cost while maintaining flexibility. With `register_scout` requiring only a wallet address and region string, the barrier to entry is minimal.

This design proposes a **verified-tier gating mechanism** combined with **off-chain KYC coordination**, shifting the Sybil attack cost while acknowledging that a purely on-chain solution cannot overcome Stellar's pseudonymous wallet model.

---

## Economic Bypass Analysis

**Current State:**
- **Pro Tier:** 0.3 XLM/month × 10 contacts/month = 0.03 XLM per contact
- **Elite Tier:** 0.7 XLM/month for unlimited contacts

**Bypass Path:**
- **N Pro wallets:** 0.3N XLM/month for 10N contacts = 0.03N XLM per contact (same per-contact cost)
- **Break-even:** 0.3N ≥ 0.7 → N ≥ 2.33 (roughly 3 wallets needed)
- **Advantage:** At N=3, pay 0.9 XLM for 30 contacts vs. 0.7 XLM for unlimited. But flexibility: can spin up wallets on-demand.
- **Cost of Registration:** Zero on-chain cost beyond XLM tx fee.

**Why This Matters:**
- Scouts with quota resistance and modest contact volume can undercut Elite's pricing indefinitely.
- The platform loses revenue and Elite tier becomes a luxury choice rather than a necessity.
- Identity-based reputation systems (player feedback, verification badges) can be gamed if scouts are truly pseudonymous.

---

## Mitigation Strategies Evaluated

### Strategy 1: On-Chain Verified-Tier Gating (Implemented)

**Design:**
1. Add a `verified: bool` flag to `ScoutProfile` in the registration contract (already exists but unused).
2. Add a new admin function `verify_scout(scout_wallet)` to set `verified = true`.
3. Gate Pro-tier subscription eligibility: only verified scouts can subscribe to Pro tier.
4. Elite tier remains always available (since Elite scouts want unlimited contacts anyway).
5. Basic tier remains always available (low-value tier, not a bypass vector).

**Trade-offs:**
- **Pros:**
  - Fully on-chain enforcement; no external dependency.
  - Clear signal: verified scouts are known to a trusted set.
  - Raises cost of Sybil attack: attacker must obtain multiple verified scout identities.
- **Cons:**
  - Verification is manual (admin-driven). Does not scale to all scouts.
  - Does not define *how* scouts get verified (KYC, social proof, payment history, etc.).
  - Addresses the *friction* but not the *identity uniqueness* problem.
  - Scouts unhappy with verification delay/friction may simply upgrade to Elite.

**Implementation:**
- Modify `register_scout` to set `verified = false` by default.
- Add admin function `verify_scout(wallet: Address)` to flip the flag.
- Modify `subscribe()` to check: if tier == Pro, require `scout.verified == true`.

### Strategy 2: Off-Chain KYC Gate (Coordination Layer)

**Design:**
- Frontend/backend enforces KYC (identity verification, proof-of-personhood, or other mechanism) *before* displaying the register/subscribe UI to scouts.
- Contract remains unaware; only already-vetted scouts ever call `register_scout`.
- By the time a scout address is used on-chain, it has been tied to a unique identity off-chain.
- Off-chain system maintains a mapping of verified identities → allowed wallet addresses.

**Trade-offs:**
- **Pros:**
  - Strongest guarantee if KYC is truly rigorous (1:1 identity → wallet binding).
  - Does not leak identity on-chain (privacy-preserving).
  - Can be upgraded without contract redeploy.
- **Cons:**
  - Requires buy-in from frontend/backend teams (outside this repo).
  - Introduces a centralized trust point (the KYC service).
  - If backend is compromised, all wallets can be registered; attacker simply bypasses UI.
  - No on-chain enforcement; contracts cannot verify scouts are "properly" registered.

**Scope:**
- Out of scope for this issue (requires coordination with ai.md backend boundaries).
- Documented as *recommended complementary approach*.

### Strategy 3: Subscription-Binding (Per-Wallet Cost Increase) - Rejected

**Design:**
- Charge a higher subscription fee for the second, third, etc. Pro subscription from the same IP or derived sender.

**Why Rejected:**
- IP-based detection is unreliable (VPNs, shared networks).
- Requires off-chain oracle coordination (not a core-contract concern).
- Doesn't prevent attacker from using different IPs, relayers, or batching services.
- Punishes legitimate multi-device users unfairly.

---

## Implementation: Verified-Tier Gating

### Data Changes

**No new types.** Existing `ScoutProfile.verified: bool` field is used.

**No new events explicitly for verification** (admin event `verify_scout` already captured via admin operations).

### New Functions

#### `verify_scout(wallet: Address) -> Result<(), ScoutChainError>` (registration contract)

Admin-only function to mark a scout as verified.

**Auth:** Admin must sign.

**Errors:** `NotInitialized`, `Unauthorized`, `ScoutNotFound`.

**Semantics:**
- Fetches the scout by wallet address.
- Sets `verified = true`.
- Extends TTL and emits a `scout_verified` event.

### Modified Functions

#### `subscribe()` (scout_access contract)

Add a gate before charging the subscription fee:

```rust
if tier == SubscriptionTier::Pro {
    if let Some(reg_contract_addr) = env
        .storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::RegistrationContract)
    {
        let reg_client = registration_contract::Client::new(&env, &reg_contract_addr);
        match reg_client.try_get_scout_by_wallet(&scout) {
            Ok(Ok(scout_profile)) => {
                if !scout_profile.verification.verified {
                    return Err(ScoutAccessError::ScoutNotVerified);
                }
            }
            _ => {
                // Scout not found in registration contract; deny Pro-tier access
                return Err(ScoutAccessError::ScoutNotVerified);
            }
        }
    }
    // If registration contract is not wired, allow Pro-tier subscription (graceful degradation)
}
```

Requires a cross-contract call to the registration contract. See "Cross-Contract Integration" below. Note that the check reads `scout_profile.verification.verified` (the structured `ScoutVerificationRecord`), not the top-level `ScoutProfile.verified` legacy bool — `verify_scout` keeps both in sync today, but the structured field is the one this gate actually depends on.

### Error Codes

**New error in scout_access contract:**
```rust
/// Scout is not verified; cannot subscribe to Pro tier.
ScoutNotVerified = 27,
```

**Reused (pre-existing) error in registration contract:** `ScoutNotFound` already existed as code `12` before this feature (used by `get_scout` and other lookups) and is reused as-is for `get_scout_by_wallet` — no new registration-contract error variant was needed:
```rust
/// Invalid `scout_id`.
ScoutNotFound = 12,
```

### Cross-Contract Integration

The scout_access contract calls the registration contract to fetch and validate scout verification status. As implemented, this uses the registration contract's existing wallet-lookup function rather than a new, verification-specific one:

1. `get_scout_by_wallet(env: Env, wallet: Address) -> Result<ScoutProfile, ScoutChainError>` (`contracts/registration/src/lib.rs`) — a general-purpose, pre-existing wallet-lookup function (its doc comment reads "Get a scout profile by wallet address. Used by scout_access contract for Pro-tier verification gating."), reused here rather than adding a verification-specific `get_scout_profile` function. It resolves the wallet to a `scout_id` via the `DataKey::ScoutByWallet` index and returns the full `ScoutProfile`, which includes the `verification: ScoutVerificationRecord` the gate inspects.

2. `scout_access`'s `subscribe()` (`contracts/scout_access/src/lib.rs`) declares a local `registration_contract` module with its own `#[contractclient]`-derived `Client` and a minimal `RegistrationContractClient` trait exposing only `get_scout_by_wallet`, plus a local copy of the `ScoutProfile`/`ScoutVerificationRecord` shapes it needs (contracts can't share Rust types directly across the WASM boundary — each side keeps a client-side mirror of the other's public interface, the same pattern `progress_contract`'s client module uses). Before charging the Pro-tier fee, `subscribe()` calls `try_get_scout_by_wallet` and checks `scout_profile.verification.verified`, returning `ScoutAccessError::ScoutNotVerified` if the scout isn't found or isn't verified. If no registration contract is wired (`DataKey::RegistrationContract` unset), the gate is skipped (graceful degradation) rather than blocking Pro-tier subscriptions outright.

This mirrors the existing progress-contract wiring (see `set_progress_contract`).

If a dedicated `get_scout_profile` function (or a narrower verification-only query) is ever wanted as the long-term API — e.g. to avoid exposing the full `ScoutProfile` over a call whose only real need is the `verified` flag — that would be a new addition to the registration contract, not a rename of `get_scout_by_wallet`. Per `docs/VERSIONING.md`'s append-only/no-removal rules for public contract functions, `get_scout_by_wallet` cannot simply be renamed or removed once shipped, regardless of how many callers it has today.

---

## Mitigation Effectiveness

### Sybil Attack Cost After Implementation

**Scenario: Attacker wants 30 contacts at Pro-tier pricing**

**Option 1: Multiple verified Pro wallets**
- Must get 3+ scout wallets verified by the admin.
- Each verification requires manual admin action.
- **Cost:** 0.9 XLM (3 × 0.3) + friction of obtaining 3 verified identities.
- **Barrier:** Admin must approve each verification; attacker can be rate-limited or blocked.

**Option 2: Upgrade to Elite (always available)**
- Single wallet, unlimited contacts.
- **Cost:** 0.7 XLM/month (cheaper than 3 Pro wallets).
- **Outcome:** Attacker simply subscribes to Elite; no bypass attempted.

**Conclusion:** On-chain gating alone raises the friction (admin involvement) but doesn't prevent someone from just paying for Elite. The real defense is *off-chain KYC coordination* — ensuring that each verified scout identity is tied to a unique person.

### Residual Risk

Even with verified-tier gating, an attacker who:
1. Obtains 3 legitimate KYC verifications (e.g., via friends, shell companies, identity brokers).
2. Gets each wallet verified on-chain by convincing the admin (e.g., socially engineered).
3. Pays 0.9 XLM/month for 30 Pro contacts.

...has still achieved the bypass, just at a higher friction cost. The on-chain mechanism is a *speed bump*, not a complete blocker.

**True defense:** Off-chain KYC that enforces 1:1 identity → verified wallet binding. This is outside the scope of the contracts repo but is the recommended complementary layer per ai.md boundaries.

---

## Cross-Repo Boundaries

Per ai.md and standard microservice patterns:

| Component | Repo | Owner | Responsibility |
|-----------|------|-------|-----------------|
| On-chain verified-tier gating | this repo | @scout-off | Reject Pro subscriptions for unverified scouts via contract enforcement |
| Off-chain KYC gate | frontend/backend | @frontend-team | Prevent UI submission of `register_scout` calls from unverified identities |
| Scout identity verification (admin) | this repo | @scout-off | Admin `verify_scout` function to mark scouts as verified |
| KYC service (identity provider) | external | e.g., Stripe, Onfido | Provide proof-of-personhood, document verification, etc. |
| Wallet tracking (if any) | frontend/backend | @frontend-team | Optional: maintain off-chain ledger of identity → wallet mappings for audit trails |

---

## Testing

### Unit Tests (this repo)

1. **test_verify_scout_admin_only** — Reject non-admin attempts to verify.
2. **test_verify_scout_updates_flag** — Verify that `verify_scout` sets `verified = true`.
3. **test_subscribe_pro_requires_verified** — Reject Pro subscription from unverified scout.
4. **test_subscribe_pro_succeeds_if_verified** — Accept Pro subscription from verified scout.
5. **test_subscribe_basic_works_unverified** — Unverified scouts can still get Basic tier (no gate).
6. **test_subscribe_elite_works_unverified** — Unverified scouts can still get Elite tier (no gate).

### Integration Tests (this repo)

- Verify a scout via admin function, then subscribe to Pro tier from a test contract.
- Attempt to subscribe to Pro tier as an unverified scout; verify rejection.

### Off-Chain Tests (frontend/backend repo)

- Verify that the register UI is gated behind KYC or identity verification flows.
- Test that unverified identities cannot proceed to the blockchain call layer.

---

## Backwards Compatibility

- Existing scouts with active Pro subscriptions are grandfathered in (no retroactive revocation).
- The `verified` field is new to gating logic; old scout profiles will have `verified = false` and must be explicitly verified by an admin to unlock Pro-tier renewals or downgrades.
- Basic and Elite tiers remain unrestricted.

---

## Future Work

1. **Automated KYC Integration:** Replace manual `verify_scout` admin calls with a contract-to-KYC-provider oracle (e.g., Chainlink).
2. **Reputation-Based Verification:** Allow scouts to self-attest membership in trusted communities (e.g., UEFA, Fédération Française de Football) via cryptographic proof.
3. **Rate Limiting per Verified Identity:** Track verified-identity → all-wallets mappings off-chain and enforce a global Pro-tier quota per identity.
4. **Payment History Signal:** Use historical subscription renewals and on-time payments as a verification signal instead of manual admin approval.

---

## Summary

**On-Chain Mitigation (This PR):**
- Gate Pro-tier subscriptions to verified scouts only.
- Raise attacker friction by requiring admin involvement.
- Leaves Basic and Elite tiers unrestricted.

**Off-Chain Mitigation (Recommended, separate PR):**
- Frontend KYC gate before `register_scout` is called.
- Enforce 1:1 identity → wallet binding via backend.

**Residual Risk:**
- An attacker with resources can still obtain multiple verified identities off-chain and register multiple Pro wallets, but at significantly higher cost and friction.
- Elite tier remains an attractive alternative (0.7 XLM unlimited vs. 0.9 XLM for 3 Pro wallets).
- Truly robust defense requires backend/frontend buy-in to implement identity-binding KYC.

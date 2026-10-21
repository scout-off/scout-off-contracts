# ScoutChain Gas-Griefing Resistance Audit

> **Issue:** #812
> **Last reviewed:** July 2026
> **Scope:** All four contracts — `registration`, `verification`, `progress`, `scout_access`

---

## Purpose

Gas griefing is an asymmetric-cost attack: an attacker spends a small, fixed amount
to force an ongoing, unbounded, or disproportionate cost onto another party (a victim
user, the platform, or every future caller of a function).

This document is explicitly **not** about absolute cost (that is tracked by the
scalability issue category). It is about the attacker/victim cost asymmetry: can a
malicious actor cheaply inflate the cost of a query or storage operation that
someone else pays for?

---

## Methodology

Each vector below is identified using the pattern:

1. **Attacker action** — what the attacker calls and what it costs them (one-time, per-call, or rate-limited).
2. **Victim action** — who pays the elevated cost and when.
3. **Cost asymmetry** — attacker cost vs victim cost, expressed in on-chain terms.
4. **Mitigation** — the cap, fee, or restructuring that bounds the asymmetry.
5. **Regression test** — test file and function that proves the mitigation holds.

Cross-reference against scalability issues: none of the mitigations below duplicate
work already tracked in the scalability issue category; they address asymmetric-cost
specifically.

---

## Vector 1: ValidatorVector Monotonic Growth

**Contract:** `verification`
**Code location:** `DataKey::ValidatorVector`, `get_validators()` in `contracts/verification/src/lib.rs`

### Exploit scenario

1. Attacker pays a one-time admin cost to get registered as a validator (admin must
   approve, so this requires colluding with or compromising the admin — low probability,
   but worth documenting).
2. Admin later revokes the attacker's validator record (`revoke_validator`).
3. The `ValidatorVector` persistent entry is **never pruned** — the revoked address
   remains in the vector forever.
4. Any caller of `get_validators()` pays an O(N) scan cost over all ever-registered
   validators, including all revoked ones, calling `get_validator_status()` on each.
5. After 100 validator churn cycles (registrations + revocations), a single
   `get_validators()` call scans 100 entries instead of the ~10 that might be active.

**Cost asymmetry:**
- Attacker: one-time validator registration (admin-gated, therefore low probability).
- Victim (every `get_validators()` caller): O(total_ever_registered) CPU instructions
  per call, growing linearly with total historical validator count.

**Estimated instruction growth:**  
At 10 active validators → ~50,000 CPU instructions for `get_validators()`.  
At 100 total (mix of active + revoked) → ~500,000 CPU instructions.  
At the cap (100 total) this is bounded.

### Mitigation

The `MAX_VALIDATORS` cap of **100** (constant in `contracts/verification/src/lib.rs`)
bounds the `ValidatorVector` length permanently. No more than 100 validators can ever
be registered (active or revoked), so the O(N) scan is bounded at O(100).

The cap is enforced in `register_validator` and `batch_register_validators` — both
return `ValidatorCapReached` (code 15) when `TotalValidatorCount >= 100`.

**Accepted residual risk:** The scan cost at N=100 is bounded but non-trivial. If
validator churn is high (many register+revoke cycles), the active count may be far
lower than 100, but the scan still costs O(100). This is an accepted design tradeoff:
the validator set is admin-gated so churn requires admin action each time.

### Regression test

`contracts/verification/tests/gas_griefing_regression.rs`  
- `test_validator_cap_enforced_at_100` — registers 100 validators, proves the 101st
  is rejected with `ValidatorCapReached`.
- `test_get_validators_with_revoked_entries_bounded` — registers 10 validators,
  revokes 5, proves `get_validators()` returns only the 5 active ones and does not
  exceed the CPU budget.

---

## Vector 2: `register_player` Spam Inflates `filter_players` Cost

**Contract:** `registration`
**Code location:** `filter_players()` in `contracts/registration/src/lib.rs`

### Exploit scenario

1. Attacker calls `register_player` many times with minimal data (minimum-length
   strings, dummy IPFS hashes). Each registration costs the attacker one transaction
   fee (~0.00001 XLM) plus the Soroban resource fee for writing a `Player(id)` entry
   and updating the `PlayersByLevelRegion` index.
2. Every registered player is added to the composite level+region index.
3. Any scout calling `filter_players` with a matching region must iterate over all
   entries in that bucket, paying O(registered_players_in_bucket) CPU instructions
   per query.
4. 1,000 spam registrations in one region inflate every scout's `filter_players`
   cost by O(1,000) — paid by the scout, not the registrant.

**Cost asymmetry:**
- Attacker: ~0.00001 XLM per registration (Stellar base transaction fee).
- Victim (every scout querying that region): O(spam_count) CPU instructions per
  call, up to the page limit.

### Mitigation

`filter_players` caps its result set at **50 entries per call** (`limit.min(50)`).
This bounds the per-call work to O(50) profile reads regardless of how many players
are in the index. Callers paginate through subsequent calls.

**Important nuance:** The index scan itself may be larger than 50 because the function
skips deactivated players and applies position/level filters before the limit is
applied. In the worst case (all registered players are in the bucket but none match
the position filter), the function scans the full bucket.

**Accepted residual risk:** A targeted spam attack flooding one specific
`(level, region)` bucket could inflate the scan cost for that bucket beyond 50
iterations before the limit applies. At realistic scale (each registration costs a
transaction fee), this requires non-trivial attacker spend. This is documented as an
accepted risk at current fee levels; a registration fee or CAPTCHA on the off-chain
layer is the correct long-term mitigation, not a contract change.

### Regression test

`contracts/registration/tests/gas_griefing_regression.rs`  
- `test_filter_players_page_limit_enforced` — registers 60 players in one bucket,
  calls `filter_players` with `limit=100`, asserts at most 50 results are returned.

---

## Vector 3: TTL-Bump Asymmetry (Scout Paying for Player Storage)

**Contract:** `scout_access`, `registration`
**Code location:** `pay_to_contact()` in `contracts/scout_access/src/lib.rs`,
  `load_stored_player()` in `contracts/registration/src/lib.rs`

### Exploit scenario

1. Player registers a profile (creates persistent storage entries for their record,
   level index, region/position index entries).
2. A scout calls `pay_to_contact(player_id)`. The scout_access contract reads the
   subscription record and writes a `ContactRecord` entry, extending both TTLs.
3. The scout also indirectly causes TTL extensions on the `PlayersByLevelRegion`
   index and `PlayerLevel` entry when those are read as part of the contact flow.
4. The scout pays the TTL extension cost for storage they did not create.

**Cost asymmetry:**
- Player (or attacker creating players): one-time registration cost.
- Scout: transaction fee + resource fee for bumping TTL on player storage on every
  `pay_to_contact` call.

### Assessment: Low severity — accepted risk

The scout's transaction fee covers the TTL extension; they are not paying on behalf
of a third party without receiving value. The scout receives the contact record and
access to the player's data in return. The cost is proportional to the benefit
received.

At current Soroban fee levels, extending a 518,400-ledger TTL costs ~100–150 CPU
instructions regardless of the entry size (see `docs/TTL_POLICY.md`). This is a
negligible fraction of the total `pay_to_contact` transaction cost.

**Mitigation:** Accepted risk. No code change required. The asymmetry is bounded by
the number of persistent keys touched per transaction, which is O(1) per contact.
If future fee schedule changes make this material, a refund mechanism (already
present as `refund_subscription` on admin) can compensate affected scouts.

---

## Vector 4: EvidenceUsed Key Accumulation

**Contract:** `verification`
**Code location:** `DataKey::EvidenceUsed(hash)`, `approve_milestone()` in
  `contracts/verification/src/lib.rs`

### Exploit scenario

1. Each `approve_milestone` call writes one `EvidenceUsed(evidence_hash)` persistent
   key that is never deleted.
2. Over time, the number of `EvidenceUsed` keys grows monotonically with total
   approved milestones.
3. These entries do not directly inflate any query's cost (they are keyed by hash and
   only checked one at a time). However, each entry incurs ongoing TTL-renewal rent.

**Cost asymmetry:**
- Validator (approving milestones): writes one permanent key per approval.
- Platform: pays ongoing TTL-renewal rent for all `EvidenceUsed` entries indefinitely.

### Mitigation

The total number of `EvidenceUsed` keys is mathematically bounded:
- `MAX_VALIDATORS` = 100 × `MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR` = 5 = 500
  entries **per player**.
- Total across all players: 500 × N_players (unbounded, but proportional to
  legitimate platform activity, not to attacker spam).

Because each milestone approval requires an authorized (admin-registered) validator,
this is not a spammable path. The growth is proportional to legitimate validator
activity.

**Accepted risk:** EvidenceUsed storage grows with legitimate milestones. At 1M
players × 3 average milestones × 100-byte entry = 300 MB — well within the
platform's expected operating cost. See `docs/STORAGE_COST_MODEL.md` for the full
cost curve.

---

## Vector 5: OutstandingTrialEscrows Index Growth

**Contract:** `scout_access`
**Code location:** `DataKey::OutstandingTrialEscrows`, `log_trial_offer()`,
  `expire_trial_offers()` in `contracts/scout_access/src/lib.rs`

### Exploit scenario

1. Elite scouts spam `log_trial_offer` for many players, each creating a
   `TrialEscrow` entry and adding it to the `OutstandingTrialEscrows` index.
2. The `OutstandingTrialEscrows` index grows unboundedly if offers are not confirmed
   or expired.
3. `expire_trial_offers(limit)` is the only path that prunes this index. If it is
   not called frequently enough, the index grows and each subsequent call to
   `expire_trial_offers` must scan up to `limit` entries.

**Cost asymmetry:**
- Attacker (spamming trial offers): each offer requires escrowing
  `trial_offer_escrow_stroops` (default 500,000 stroops = 0.05 XLM) and having an
  active Elite subscription. This is an expensive attack.
- Platform (calling `expire_trial_offers`): O(limit) scan, capped at 20 per call by
  `EXPIRE_TRIAL_OFFERS_MAX_LIMIT`.

### Mitigation

`expire_trial_offers` is capped at `EXPIRE_TRIAL_OFFERS_MAX_LIMIT = 20` entries per
call (see `contracts/scout_access/src/lib.rs`). The per-call cost is O(20)
regardless of backlog size. The per-(scout, player) 24-hour cooldown
(`TRIAL_OFFER_COOLDOWN_SECS = 86_400`) severely rate-limits offer spam.

**Accepted residual risk:** A wealthy attacker with many Elite subscriptions could
still build a large backlog. At 0.05 XLM per offer this requires significant capital.
Documented as accepted at current fee levels.

---

## Cross-Reference with Scalability Issues

| Vector | This audit (griefing/asymmetric cost) | Scalability issue (absolute cost) |
|--------|--------------------------------------|-----------------------------------|
| ValidatorVector growth | Cap at 100 bounds O(N) scan victim cost | Separate: upgrading cap requires contract upgrade |
| filter_players spam | Per-call limit=50 bounds each scout's cost | Separate: index restructuring for O(1) lookups |
| TTL-bump asymmetry | Accepted (scout pays for own contact benefit) | Separate: TTL policy redesign tracked in #705 |
| EvidenceUsed growth | Bounded by validator × milestone caps | Separate: storage cost model in STORAGE_COST_MODEL.md |
| TrialEscrow backlog | EXPIRE_TRIAL_OFFERS_MAX_LIMIT + cooldown | Separate: ring-buffer redesign tracked separately |

No work in this audit duplicates the remediation tracked in the scalability issue category.

---

## Summary

| Vector | Severity | Status |
|--------|----------|--------|
| ValidatorVector O(N) scan | Medium | **Mitigated** — capped at 100 |
| register_player spam → filter_players | Medium | **Mitigated** — per-call limit 50 |
| TTL-bump asymmetry | Low | **Accepted risk** |
| EvidenceUsed key accumulation | Low | **Accepted risk** — bounded by caps |
| OutstandingTrialEscrows backlog | Low-Medium | **Mitigated** — rate-limit + sweep cap |

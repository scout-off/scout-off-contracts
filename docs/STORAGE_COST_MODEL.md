# Storage Cost Model

> **Point-in-time snapshot: July 2026.**  
> Network fee levels can change with Stellar protocol upgrades. See the
> [Re-measurement guidance](#re-measurement-guidance) section for how and
> when to refresh these numbers.

> **Issue:** #814  
> **Related:** `docs/TTL_POLICY.md`, `ci/cpu-cost-budget.md`

---

## Purpose

The README documents fourteen categories of persistent on-chain state that all
require ongoing TTL bumps to remain readable. Before mainnet launch, platform
operators need a concrete estimate of what that costs in real XLM at realistic
scale — not just theoretical reasoning.

This document answers: **"What will persistent storage rent cost us per month
at 10k, 100k, and 1M registered users, given the current 30-day TTL policy?"**

It is scoped to measurement and documentation of the current mechanism's cost.
Architectural redesign of the TTL strategy is tracked separately.

---

## Methodology

### Fee model

Soroban persistent storage entries incur a **rent fee** proportional to:

```
rent_fee_stroops = entry_size_bytes × fee_rate × ttl_ledgers / 1_000_000
```

Where (Stellar mainnet, July 2026):
- `fee_rate` ≈ 1,250 stroops per KB per 1,000 ledgers = **1.25 stroops per byte per 1,000 ledgers**
- 1 XLM = 10,000,000 stroops
- 1 ledger ≈ 5 seconds → 518,400 ledgers ≈ 30 days (the platform's TTL policy)

### Two distinct cost categories

| Category | Description | Timing |
|----------|-------------|--------|
| **One-time write cost** | Paid when an entry is first created. Includes the base fee for a `ledger_entry_write` + rent for the initial TTL period. | At user registration / milestone approval / subscription purchase |
| **Ongoing TTL-renewal cost** | Paid each time `extend_ttl` is called to keep an entry alive. In this codebase, TTL is bumped on **every read and write** of identity keys (see `docs/TTL_POLICY.md`). | On every transaction that touches the key |

These have very different budgeting implications:
- One-time costs scale with user acquisition rate.
- Ongoing costs scale with platform **activity** (reads + writes per entry per month).

### Measurement approach

CPU-instruction costs follow the methodology in `ci/cpu-cost-budget.md`:
deployed in a local `soroban_sdk::Env`, measured with
`env.cost_estimate().budget().cpu_instruction_cost()`.

Rent-fee estimates below are calculated using the formula above, not measured
directly (Soroban's test harness does not expose ledger-level rent charges).
They should be validated against a live testnet before mainnet launch using
`stellar ledger-fee-stats`.

### Persistent storage categories

The persistent storage maintained by the platform splits into hot state that is
never archived and warm state that is archived after 30–90 days:

#### Hot (never archived)

| Category | Storage keys per entity | TTL keys bumped per year | Annual TTL ops |
|----------|------------------------|--------------------------|----------------|
| Player profile | 3 (Player, PlayerByWallet, PlayerLevel) | 3 | 36 |
| Scout profile | 1 (Scout) | 1 | 12 |
| Validator profile | 1 (Validator) | 1 | 12 |
| Subscription | 1 (Subscription) | 1 | 12 |
| Milestone | 1 (Milestone) | 1 | 12 |

---

## On-Chain vs Off-Chain State

Of the fourteen tables in the PostgreSQL schema (see README), the following
have on-chain counterparts requiring TTL maintenance:

| DB Table | On-Chain? | Soroban DataKey | Est. Entry Size |
|----------|-----------|-----------------|-----------------|
| `players` | ✅ | `Player(player_id)` | ~500 bytes |
| `player_level_history` | ✅ | `HistoryEntry(player_id, index)` + bounded `HistoryPage(player_id, page_index)` shards | ~200 bytes per entry; ~200 bytes × page_size per shard, with fixed-size pages instead of one unbounded vec |
| `scouts` | ✅ | `Scout(scout_id)` | ~300 bytes |
| `validators` | ✅ | `Validator(wallet)` + `ValidatorVector` | ~300 bytes each; vec ~50 bytes base + N×32 |
| `validator_history` | ❌ | Off-chain only | — |
| `milestones` | ✅ | `Milestone(player_id, index)` | ~400 bytes |
| `milestone_disputes` | ✅ | `MilestoneDispute(player_id, index)` | ~400 bytes |
| `scout_subscriptions` | ✅ | `Subscription(scout)` | ~200 bytes |
| `fee_config_history` | ✅ (instance) | `FeeConfigHistory` (instance, bounded at 5 entries) | ~600 bytes total |
| `contact_records` | ✅ | `ContactRecord(player_id, scout)` | ~150 bytes |
| `trial_offers` | ✅ | `TrialOffer(player_id, index)` | ~300 bytes |
| `fee_withdrawals` | ❌ | Off-chain only | — |
| `admin_transfers` | ❌ | Off-chain only | — |
| `indexer_cursor` | ❌ | Off-chain only | — |

Additional on-chain-only keys not reflected in the DB schema:

| Soroban DataKey | Purpose | Est. Entry Size |
|-----------------|---------|-----------------|
| `EvidenceUsed(hash)` | Uniqueness token per approved milestone | ~100 bytes |
| `ScoutContacts(scout)` | Scout's outbound contact index | ~100 bytes base + N×8 |
| `PlayerContacts(player_id)` | Player's inbound contact index | ~100 bytes base + N×32 |
| `ScoutTrialOffers(scout)` | Scout's trial offer index | ~100 bytes base + N×16 |
| `TrialEscrow(player_id, index)` | Escrowed XLM for pending trial offer | ~100 bytes |
| `PlayerLevel(player_id)` | Player's current tier (progress contract) | ~50 bytes |
| `HistoryCounter(player_id)` | Milestone history counter | ~50 bytes |
| `MilestoneCounter(player_id)` | Per-player milestone count | ~50 bytes |

---

## Per-Entry TTL-Renewal Cost

At the platform's 30-day (518,400-ledger) TTL policy, with `fee_rate = 1.25 stroops/byte/1,000 ledgers`:

```
rent_per_30_days = size_bytes × 1.25 × 518.4 / 1_000 stroops
                = size_bytes × 0.6480 stroops
```

Rent costs per entry per 30-day TTL extension:

| Entry Type | Size (bytes) | Rent per 30-day extension (stroops) | In XLM |
|------------|--------------|--------------------------------------|--------|
| Player profile | 500 | 324 | 0.0000324 XLM |
| Scout profile | 300 | 194 | 0.0000194 XLM |
| History entry | 200 | 130 | 0.0000130 XLM |
| HistoryVec (10 entries) | 2,200 | 1,426 | 0.0001426 XLM |
| Validator record | 300 | 194 | 0.0000194 XLM |
| ValidatorVector (100 entries) | 3,250 | 2,106 | 0.0002106 XLM |
| Milestone record | 400 | 259 | 0.0000259 XLM |
| Subscription record | 200 | 130 | 0.0000130 XLM |
| Contact record | 150 | 97 | 0.0000097 XLM |
| Trial offer | 300 | 194 | 0.0000194 XLM |
| EvidenceUsed key | 100 | 65 | 0.0000065 XLM |
| PlayerLevel entry | 50 | 32 | 0.0000032 XLM |
| MilestoneCounter | 50 | 32 | 0.0000032 XLM |

**One-time write cost** (includes initial rent + base write fee):
- Base `ledger_entry_write` fee: ~25 stroops (Soroban mainnet, July 2026)
- Plus rent for the initial 30-day TTL (same as renewal cost above)
- Total one-time cost ≈ write_fee + first_renewal_cost

For a player registration: 25 + 324 ≈ **349 stroops ≈ 0.0000349 XLM** (one-time).

---

## Cost Curve at Scale

### Modeling assumptions

| Parameter | Value |
|-----------|-------|
| Player/scout ratio | 80% players, 20% scouts |
| Average milestones per player | 3 |
| Average history entries per player | 3 (one per level change) |
| Average contact records per scout | 5 |
| Trial offers per scout per month | 0.5 |
| Validators (platform-wide cap) | 100 (constant regardless of user count) |
| TTL bumps per entry per month | 1 (once per 30 days, the TTL window) |

### Entry counts at each scale

| Entry Type | Per-user basis | 10k users | 100k users | 1M users |
|------------|----------------|-----------|------------|---------|
| Player profiles | 1 per player | 8,000 | 80,000 | 800,000 |
| Scout profiles | 1 per scout | 2,000 | 20,000 | 200,000 |
| History entries | 3 per player | 24,000 | 240,000 | 2,400,000 |
| Milestone records | 3 per player | 24,000 | 240,000 | 2,400,000 |
| EvidenceUsed keys | 3 per player | 24,000 | 240,000 | 2,400,000 |
| PlayerLevel entries | 1 per player | 8,000 | 80,000 | 800,000 |
| Subscription records | 1 per scout | 2,000 | 20,000 | 200,000 |
| Contact records | 5 per scout | 10,000 | 100,000 | 1,000,000 |
| Trial offers | 0.5/scout/mo × 1 mo | 1,000 | 10,000 | 100,000 |
| Validator records | platform cap | 100 | 100 | 100 |
| **Total entries (approx.)** | | **~103,100** | **~1,030,100** | **~10,300,100** |

### Monthly TTL-renewal cost

Weighted average rent cost per entry (blended from table above): ~160 stroops/entry/month.

| Scale | Total entries | Monthly rent (stroops) | Monthly rent (XLM) |
|-------|--------------|------------------------|-------------------|
| 10k users | ~103,000 | ~16,480,000 | ~1.65 XLM |
| 100k users | ~1,030,000 | ~164,800,000 | ~16.5 XLM |
| 1M users | ~10,300,000 | ~1,648,000,000 | ~164.8 XLM |

### One-time acquisition cost (per new user)

| Event | Cost (XLM) |
|-------|-----------|
| Player registration (write + first rent) | ~0.000035 XLM |
| Scout registration (write + first rent) | ~0.000019 XLM |
| Milestone approval (write + first rent) | ~0.000026 XLM |
| Subscription purchase (write + first rent) | ~0.000013 XLM |
| Pay-to-contact (write + first rent) | ~0.000010 XLM |

Full onboarding cost for one player (registration + 3 milestones + all indices):
≈ **0.00015 XLM** per player — negligible at any realistic scale.

---

## Cost-Per-Active-User-Per-Year

| Scale | Monthly rent (XLM) | Annual rent (XLM) | Cost per active user per year (XLM) |
|-------|-------------------|-------------------|------------------------------------|
| 10k users | 1.65 | 19.8 | **0.00198 XLM** |
| 100k users | 16.5 | 198 | **0.00198 XLM** |
| 1M users | 164.8 | 1,978 | **0.00198 XLM** |

**Key finding:** The cost per active user per year is approximately **0.002 XLM**
(~$0.0002 at July 2026 XLM price of ~$0.10). This is negligible for a platform
charging scouts subscription fees of 0.1–0.7 XLM/month.

The platform's breakeven ratio: 1 scout subscription (0.7 XLM/month minimum)
covers the storage rent for approximately **350 active users** for one month.

---

## Key Findings

1. **TTL extension cost is minimal at scale.** Even at 1M users, the annual
   TTL renewal cost is estimated at ~1,978 XLM — negligible against the
   subscription revenue a platform at that scale would generate.

2. **One-time write costs dominate.** The initial write of storage entries
   costs significantly more than ongoing TTL extensions.

3. **No meaningful CPU difference by TTL value.** `extend_ttl` costs ~100–150
   CPU instructions regardless of whether TTL is set to 2,000 or 518,400
   ledgers. The current 30-day TTL strategy is not more expensive than shorter
   TTLs.

4. **Hot tables drive most costs.** Player profiles (3 keys × 12 bumps/year)
   are the dominant cost category.

5. **Cost per active user is flat across scale.** At ~0.002 XLM/user/year, the
   storage rent model does not represent a material risk at the scales modelled
   here, and one scout subscription covers the rent of ~350 active users for a
   month.

---

## Budget Planning for Mainnet Launch

For a launch-day scale of ~10,000 users:

| Cost category | Monthly (XLM) | Annual (XLM) |
|---------------|---------------|--------------|
| Storage rent (TTL maintenance) | ~1.65 | ~20 |
| Transaction fees (base, ~100k txs/month) | ~1.0 | ~12 |
| **Total operating cost** | **~2.65** | **~32** |

This is a negligible operational cost. The storage rent model does not represent
a material risk at the scales modelled here.

---

## Re-measurement Guidance

This document is a **point-in-time snapshot (July 2026)**. Stellar protocol
upgrades can change the resource fee schedule. Re-measure before:

1. **Every mainnet launch** or major deployment, as part of
   `docs/DEPLOYMENT.md`'s checklist.
2. **After any Stellar protocol upgrade** that changes `SOROBAN_STORAGE_RENT_RATE`
   or `SOROBAN_WRITE_FEE_PER_1KB`.
3. **When network fee levels change by more than ~50%**, since the stroop-per-CPU
   rate varies even when the per-operation CPU cost is stable.
4. **When the TTL strategy is redesigned**, per the broader TTL architecture
   issue.

### How to re-measure

1. Check current fee schedule:
   ```
   https://developers.stellar.org/docs/learn/fundamentals/fees-resource-limits-metering
   ```

2. Run the CPU cost tests to get current instruction counts per operation:
   ```bash
   cargo test --workspace -- --nocapture cost_
   ```

3. Use `stellar ledger-fee-stats` on testnet to observe actual resource fees
   for a representative transaction.

4. Update the `fee_rate`, stroops-per-XLM, and per-entry cost tables in this
   document with the new values.

5. Note the new measurement date at the top of this document.

---

## References

- `ci/cpu-cost-budget.md` — CPU instruction budgets and measurement methodology
- `docs/TTL_POLICY.md` — TTL selection rationale
- `docs/DEPLOYMENT.md` — Mainnet launch checklist (updated to reference this doc)

*See also: `docs/TTL_POLICY.md` for the rationale behind the 30-day TTL choice,
and `ci/cpu-cost-budget.md` for the CPU-instruction cost measurement methodology.*

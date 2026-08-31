# Trial Escrow Economic Impact Analysis

> **This is a prioritization/impact analysis document, not a code fix.**
> Its purpose is to turn a qualitative "this seems risky" into a concrete
> number that decision-makers can act on.
>
> All projections are model-generated estimates based on the documented
> platform fee schedule. They should be revisited once real usage data exists.

---

## Background

### The confirmed code gap (status: fixed)

> **Update:** this section originally described a state where `log_trial_offer`
> logged trial offers without collecting any escrow, so `trial_offer_escrow_stroops`
> was purely aspirational and no capital was actually at risk. That has since
> shipped (#795): `log_trial_offer` now collects the escrow via a real token
> transfer, `contracts/scout_access/src/types.rs` defines `TrialEscrow` and
> `trial_offer_escrow_stroops` is a live `FeeConfig` field, and
> `expire_trial_offers` is an implemented, bounded sweep (see
> `docs/GAS_GRIEFING_AUDIT.md`) rather than a no-op stub. The original risk
> analysis and recommendations below remain valid — they just now describe a
> present, not a hypothetical, condition.

The live `log_trial_offer` implementation in `contracts/scout_access/src/lib.rs`
logs a trial offer on-chain, collects `trial_offer_escrow_stroops` from the
scout into a `TrialEscrow` record, and advances a player to `EliteTier`.
Unconfirmed offers are released by three shipped paths: `confirm_trial_offer`'s
late-expiry branch, the admin-run `expire_trial_offers` sweep, and the
operation-targeted `admin_refund_trial_escrow` escape hatch for a specific
identified stuck entry.

This document is deliberately written as an impact analysis against the shipped
behavior: the economic model remains useful, but the risk assumptions are now
validated against actual code paths rather than imagined future ones.

This document cross-references:

- **expire_trial_offers implementation** — the bounded sweep that refunds stale
  trial escrow after the configured expiry window.
- **TrialEscrow enumeration-index** — enumerating locked escrow entries so admin
  tooling can identify and recover stuck entries in practice.

---

## Fee Baseline

The documented example fee configuration from `docs/CONTRACT_REFERENCE.md`
and the `initialize` call in the README:

| Fee field | Stroops | XLM equivalent |
|-----------|---------|----------------|
| `contact_fee_stroops` | 100,000 | 0.01 XLM |
| `basic_sub_stroops` | 1,000,000 | 0.10 XLM |
| `pro_sub_stroops` | 3,000,000 | 0.30 XLM |
| `elite_sub_stroops` | 7,000,000 | 0.70 XLM |

The currently shipped fee model includes a live `trial_offer_escrow_stroops`
field and a `trial_offer_expiry_secs` window in `FeeConfig`. In the 0.05 XLM
example configuration, `log_trial_offer` locks 500,000 stroops per offer, and
expired entries can be swept by `expire_trial_offers` or manually resolved by
`admin_refund_trial_escrow`.

For this analysis we model the same economic range against the shipped behavior:

| Scenario | Trial escrow per offer | Rationale |
|----------|----------------------|-----------|
| Low escrow | 1,000,000 stroops (0.1 XLM) | Nominal commitment signal |
| High escrow | 10,000,000 stroops (1.0 XLM) | Meaningful anti-spam bond |

---

## Model Assumptions

The following assumptions drive all projections. They are explicitly stated so
they can be corrected when real platform usage data exists.

| Parameter | Value | Notes |
|-----------|-------|-------|
| Monthly trial offers (ramp phase) | 50 | Early platform with limited scouts |
| Monthly trial offers (growth phase) | 500 | Platform reaching traction |
| Monthly trial offers (scale phase) | 2,000 | Regional scale |
| Never-confirmed rate — Optimistic | 10% | Most scouts and players follow through |
| Never-confirmed rate — Expected | 30% | Industry norm: significant drop-off after initial interest |
| Never-confirmed rate — Pessimistic | 60% | High churn, scouts using the platform speculatively |

A "never-confirmed" offer is one where neither `confirm_trial_offer` nor
`expire_trial_offer` is ever called — the offer is recorded on-chain and the
associated escrow is locked permanently under the current code.

**The platform is modelled as starting small (50 offers/month) and growing
linearly to 500 by month 12 and 2,000 by month 24.**

---

## Simulation: Cumulative Locked Escrow

### Locked offer count (never confirmed, cumulative)

| Horizon | Optimistic (10%) | Expected (30%) | Pessimistic (60%) |
|---------|-----------------|---------------|-------------------|
| 6 months | ~113 offers | ~338 offers | ~675 offers |
| 12 months | ~413 offers | ~1,238 offers | ~2,475 offers |
| 24 months | ~2,663 offers | ~7,988 offers | ~15,975 offers |

> **Derivation**: monthly offers grow linearly from 50 to 500 over months 1–12
> and 500 to 2,000 over months 13–24. The never-confirmed fraction accumulates
> each month.

### Locked XLM at low escrow (0.1 XLM per offer)

| Horizon | Optimistic | Expected | Pessimistic |
|---------|-----------|---------|------------|
| 6 months | **~11 XLM** | **~34 XLM** | **~68 XLM** |
| 12 months | **~41 XLM** | **~124 XLM** | **~248 XLM** |
| 24 months | **~266 XLM** | **~799 XLM** | **~1,598 XLM** |

### Locked XLM at high escrow (1.0 XLM per offer)

| Horizon | Optimistic | Expected | Pessimistic |
|---------|-----------|---------|------------|
| 6 months | **~113 XLM** | **~338 XLM** | **~675 XLM** |
| 12 months | **~413 XLM** | **~1,238 XLM** | **~2,475 XLM** |
| 24 months | **~2,663 XLM** | **~7,988 XLM** | **~15,975 XLM** |

> At time of writing, XLM trades between $0.09–$0.12 USD. At $0.10 USD/XLM,
> the pessimistic 24-month high-escrow scenario represents **~$1,598 USD** in
> permanently locked capital. At a higher price ($0.30 USD/XLM) this becomes
> **~$4,793 USD**.
>
> These are not large absolute numbers for a funded protocol, but they grow
> monotonically with no recovery path and represent capital scouts have paid
> that they cannot recover — a trust and user-experience problem that
> compounds over time.

---

## Sensitivity to Escrow Value

The escrow fee is the most impactful variable. The table below shows locked
XLM at 24 months under the Expected (30%) never-confirmed rate for a range
of potential escrow values:

| Escrow per offer | Locked XLM @ 24 months (expected) |
|-----------------|-----------------------------------|
| 500,000 stroops (0.05 XLM) | ~399 XLM |
| 1,000,000 stroops (0.10 XLM) | ~799 XLM |
| 5,000,000 stroops (0.50 XLM) | ~3,994 XLM |
| 10,000,000 stroops (1.00 XLM) | ~7,988 XLM |
| 50,000,000 stroops (5.00 XLM) | ~39,938 XLM |

Even a modest 0.10 XLM escrow produces nearly 800 XLM of permanently locked
capital over 24 months at a 30% never-confirmed rate. A more meaningful
1 XLM bond approaches 8,000 XLM.

---

## Recommendation

### 1. Monitor the live escrow footprint against real usage

This analysis supports **continuing to monitor the live escrow footprint** as the
platform scales, rather than treating it as a deferred code fix. The shipped
`trial_offer_escrow_stroops` and `trial_offer_expiry_secs` settings are real
capital controls, and the 24-month expected-scenario numbers (800–8,000 XLM
depending on escrow amount) are large enough to matter for scout trust and
operational risk if never-confirmed rates remain elevated.

The important operational point is that the system now has a bounded sweep and
an explicit recovery path, so the right question is not "is the fix missing?"
but "what is the observed volume and failure rate in production?"

### 2. Keep `admin_refund_trial_escrow` as an operational safeguard

`admin_refund_trial_escrow(player_id: u64, offer_index: u32, to: Address)` is
shipped and remains a useful admin-only safety valve analogous to
`refund_subscription`. It lets operations directly resolve one specific,
identified stuck `TrialEscrow` entry without waiting for a generic
`expire_trial_offers` sweep to reach it. It rejects any target that is not
currently outstanding (already confirmed, already expired/refunded, or never
logged) and removes the entry from `OutstandingTrialEscrows` on success, so
neither a later sweep nor a late `confirm_trial_offer` can act on it again.

This is best framed as a targeted operational recovery tool, not a substitute
for monitoring the backlog of expired offers and the configured expiry window.

### 3. Treat the escrow config as active, validated, and auditable

The premise behind the original concern is no longer hypothetical:
`trial_offer_escrow_stroops` is a live `FeeConfig` field, `trial_offer_expiry_secs`
is a live expiry configuration, and `log_trial_offer` collects the escrow via an
actual token transfer. The release paths this analysis depends on —
`expire_trial_offers` and `admin_refund_trial_escrow` — are both implemented and
can be validated against live usage data.

---

## Caveats and Invitation to Correct

- All volume projections are illustrative. Real platform growth may be faster,
  slower, or non-linear.
- The never-confirmed rate is the most uncertain variable. A platform with
  strong notifications, a mobile app, and engaged scouts may achieve < 10%.
  A platform used primarily by exploratory scouts may exceed 60%.
- XLM price volatility means USD impact estimates could differ substantially
  from the values shown here.
- The model does not account for scouts who abandon wallets entirely (their
  escrow is locked regardless of the expire mechanism unless admin can recover
  it manually).

**Once real platform data is available, replace the assumption table above
with measured values and re-run the projections against the live fee settings and
truly observed expiry/sweep behavior.**

---

## Related Issues

| Issue | Description |
|-------|-------------|
| `expire_trial_offers` implementation | Code-level fix: implement the sweep function that refunds unconfirmed trial escrow after a configurable expiry window |
| TrialEscrow enumeration-index | Build an index that makes stuck-escrow enumeration practical for both the sweep function and admin tooling |

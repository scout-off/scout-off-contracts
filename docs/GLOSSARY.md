# ScoutChain Glossary

Domain-specific terms used throughout the contracts, documentation, and SDKs.
Each definition includes a role description and links to the relevant contract
functions in [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md).

---

## Attestation

An independent vote by one validator toward a **k-of-n threshold** milestone
claim, cast via `attest_milestone`. This is the alternate, multi-validator
commit path alongside the original single-validator `approve_milestone` path
(see [Milestone](#milestone)): once the admin calls
`set_milestone_threshold(n)` with `n >= 2`, `approve_milestone` (and the
off-chain-signed relay `submit_attested_milestone`) stop working, and a
milestone claim — identified by `(player_id, evidence_hash)`, not by its
description — only commits once `threshold` distinct active validators have
each independently attested to it within the current voting round. A vote
arriving after `voting_window_secs` has elapsed since the round started
starts a fresh round instead of counting toward the old tally. The default
threshold is `1`, which reproduces `approve_milestone`'s original
single-signature behaviour unchanged, so existing integrations keep working
until an operator deliberately opts into k-of-n mode.

- See [CONTRACT_REFERENCE.md — k-of-n threshold milestone
  attestation](CONTRACT_REFERENCE.md#k-of-n-threshold-milestone-attestation)
  for the full design rationale (claim identity, bounded storage, revoke-mid-vote
  and window-expiry semantics).
- Relevant functions: `attest_milestone`, `submit_attested_milestone`,
  `set_milestone_threshold`, `get_milestone_threshold`, `get_pending_claim`,
  `has_attested` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#verification).

---

## Auto-Renewal

A mechanism that lets a scout's subscription be renewed automatically when it
expires, without the scout signing each renewal transaction themselves. The
scout opts in via `set_auto_renew(scout, enabled)` (emitting the
`auto_renew_set` event); after that, any keeper — an off-chain cron job, bot,
or the scout — can call `renew_if_due(scout)` when the subscription is due,
emitting `subscription_auto_renewed` on success.

Each renewal anchors the new `expires_at` to
`max(old_expires_at, now) + sub_duration_secs` rather than
`now + sub_duration_secs`, so consecutive on-time-ish renewals produce
contiguous non-overlapping periods. The same anchored `expires_at` defines
the `pro_contact_limit` period boundary, keeping that reset aligned with the
coverage period. Renewals are due within the
`renewal_window_secs = sub_duration_secs / 10` grace window (3 days for the
30-day default); the window is floored at 1 second, so the minimum sensible
`sub_duration_secs` is 10 seconds (any shorter duration makes the computed
window 0 and relies on the clamp).

If auto-renewal is not enabled for a scout, `renew_if_due` returns the
`AutoRenewNotEnabled` error (`ScoutAccessError` code 28).

- Relevant functions: `set_auto_renew`, `renew_if_due`, `get_auto_renew` — see
  [CONTRACT_REFERENCE.md — scout_access](CONTRACT_REFERENCE.md#scout_access).
- Related: [Subscription Tier](#subscription-tier),
  [Sybil Resistance](#sybil-resistance).

---

## CID (Content Identifier)

A self-describing content hash produced by IPFS or Arweave. CIDs are stored
on-chain as strings inside player profiles and milestone evidence fields so
that off-chain video and photo assets can be retrieved and verified without
trusting a centralised server.

- IPFS CIDs start with `Qm…` (CIDv0) or `bafy…` (CIDv1).
- The `evidence_hash` parameter of `approve_milestone` and the `details_hash`
  parameter of `log_trial_offer` both accept CIDs.
- Relevant functions: `register_player`, `update_profile`, `approve_milestone`,
  `log_trial_offer` — see [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md).

---

## Contact Fee

A micro-payment in XLM (denominated in stroops) that a scout pays to unlock a
specific player's full contact details. Controlled by the `contact_fee_stroops`
field in [`FeeConfig`](#feeconfig).

- Relevant function: [`pay_to_contact`](CONTRACT_REFERENCE.md#pay_to_contactscout-address-player_id-u64---resultscoutaccesserror).

---

## ContactRecord

A record of a paid contact attempt from a scout to a player, stored by the
`scout_access` contract after successful payment of the configured contact fee.
A `ContactRecord` links the paying scout, the contacted player, and the paid
fee amount, and enables the platform to enforce repeated-contact and contact
history checks.

- Relevant functions: `pay_to_contact`, `get_contact_record`, `has_contacted`
  — see [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#scout_access).

---

## Evidence Access Grant

An append-only on-chain record (`EvidenceAccessGrant`) that a scout paid to
contact a specific player, and is therefore entitled to request that player's
off-chain evidence key-wrap from the key-wrapping service. The grant captures
the scout, the player, the ledger time it was issued, and the scout's
subscription tier **at the moment of the grant** — it is a historical fact
about a paid contact, not a live re-derivation of the scout's current
entitlement.

One grant is written per successful `(player_id, scout)` contact, atomically
with the contact-fee transfer and `ContactRecord` write, on every successful
`pay_to_contact` (and each newly recorded contact in `batch_contact_players`).
It is unreachable on any rejected call, because it runs after every
subscription-tier, quota, and payment guard has passed.

A grant is **append-only and is not auto-revoked** by a subscription lapse,
downgrade, or expiry — those code paths never touch grant state, so a scout who
paid while subscribed keeps access even after downgrading. The only way to
revoke is the explicit, admin-gated `admin_revoke_evidence_access`, which sets
`revoked = true` / `revoked_at` but never deletes the record (the audit trail
stays intact) and is idempotent. Revocation only gates *future* key-wrap
requests; it cannot claw back a key already delivered off-chain.

- Relevant functions: `has_evidence_access`, `get_evidence_access_grant`,
  `get_player_access_grants`, `admin_revoke_evidence_access` (grant is written
  by `pay_to_contact` / `batch_contact_players`) — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#scout_access).
- Relevant events: `evidence_access_granted`, `evidence_access_revoked`.
- Full design and append-only rationale:
  [EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md).
- Related: [ContactRecord](#contactrecord),
  [Subscription Tier](#subscription-tier).

---

## FeeConfig

The primary configuration struct for the `scout_access` contract. Controls all
subscription and pay-to-contact fee rates. Set at `initialize` time and
adjustable via `update_fee_config`.

| Field | Type | Unit | Valid Range | Typical Value |
|---|---|---|---|---|
| `contact_fee_stroops` | `i128` | stroops (1 XLM = 10 000 000 stroops) | > 0 | `100000` (0.01 XLM) |
| `basic_sub_stroops` | `i128` | stroops | > 0 | `1000000` (0.1 XLM) |
| `pro_sub_stroops` | `i128` | stroops | > 0 | `3000000` (0.3 XLM) |
| `elite_sub_stroops` | `i128` | stroops | > 0 | `7000000` (0.7 XLM) |
| `sub_duration_secs` | `u64` | duration in seconds (not a Unix timestamp) | > 0 | `2592000` (30 days) |
| `pro_contact_limit` | `u32` | count | > 0 | `10` (10 contacts/period) |
| `trial_offer_escrow_stroops` | `i128` | stroops | > 0 | `500000` (0.05 XLM) |
| `trial_offer_expiry_secs` | `u64` | duration in seconds | > 0 | `3600` (1 hour) |

All fields must be strictly greater than zero; `initialize` and
`update_fee_config` return `InvalidInput` otherwise.

`pro_contact_limit` caps the number of unique players a **Pro-tier** scout
may contact in a single subscription period. Reaching the limit causes
`pay_to_contact` to return `ProContactLimitReached` (code 20). **Elite-tier
scouts are exempt** from this cap.

`trial_offer_escrow_stroops` is the XLM amount held in escrow when a scout
logs a trial offer via `log_trial_offer`. The escrowed amount is released to
the contract's accumulated fees on successful `confirm_trial_offer`, or
refunded to the originating scout if the offer expires (confirmed after
`trial_offer_expiry_secs` have elapsed, or swept by `expire_trial_offers`).

`trial_offer_expiry_secs` defines the window (in seconds) within which a
player must call `confirm_trial_offer` after the offer was logged. After this
window the confirmation path refunds the scout's escrow and emits
`trial_offer_expired`.

- Relevant functions: `initialize`, `update_fee_config`, `get_fee_config` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#scout_access).

---

## Migration Window

An admin-toggled instance flag on each contract that gates all `admin_seed_*` state-seeding functions. When the migration window is open (`migration_window_is_open` returns `true`), operators may call the seeding entrypoints to replay exported state onto a freshly deployed contract without requiring the original wallet signatures. When the window is closed, all `admin_seed_*` calls are rejected with `MigrationNotActive`.

The window is opened with `open_migration_window` and closed with `close_migration_window` (both admin-only). It should be kept open only for the duration of a controlled replay, then closed before the new contract begins serving real traffic.

Affected seeding functions (present on all four contracts):

- `registration`: `admin_seed_player`, `admin_seed_scout`
- `verification`: `admin_seed_validator`, `admin_seed_milestone`
- `progress`: `admin_seed_history`
- `scout_access`: `admin_seed_subscription`, `admin_seed_contact`

Tooling: `scripts/replay-state.sh` opens the window, seeds all replayable data categories, then closes it. `scripts/migrate-contract.sh` orchestrates the full migration including this step.

- See [docs/MIGRATION_GAPS.md](MIGRATION_GAPS.md) for the canonical list of which data categories are fully, partially, or not replayable.
- See [docs/DEPLOYMENT.md — Address migration](DEPLOYMENT.md#address-migration-new-contract-id) for the step-by-step migration procedure.

---

## Milestone

A verified player achievement recorded on-chain by an authorised validator.
Each milestone stores a plain-text description, an IPFS/Arweave evidence CID,
the approving validator's address, and a ledger sequence number for
auditability.

Examples: "Scored 5 goals in Local Cup", "Top speed clocked at 32 km/h".

A milestone commits through one of two paths: the original single-validator
`approve_milestone` (one signature is enough), or, once the admin opts into
k-of-n mode via `set_milestone_threshold(n >= 2)`, the multi-validator
[Attestation](#attestation) path (`attest_milestone`), which requires several
independent validators to corroborate the same claim before it commits. Only
one path is active at a time — see [Attestation](#attestation) for the
mechanics.

- Relevant functions: `approve_milestone`, `attest_milestone`,
  `get_milestone`, `get_milestone_count` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#verification).

---

## Milestone Dispute

A formal on-chain challenge raised by a player against a specific milestone
that was approved for their profile. Only the affected player may file a
dispute — validators and scouts have no standing to do so.

A dispute is routed to one of two resolution paths at filing time, based on
the `impact_score` recorded on the dispute versus `jury_config.impact_threshold`
(default `100`, admin-configurable via `set_jury_config`):

| Route | Condition | Resolution |
|---|---|---|
| Admin-only | `impact_score < impact_threshold` | The platform admin calls `resolve_dispute` |
| Jury | `impact_score >= impact_threshold` | Active validators call `cast_dispute_vote`; `tally_dispute` finalizes the outcome. `resolve_dispute` is blocked for these disputes (`DisputeRequiresJury`). |

A dispute record carries two outcome fields:

| Field | Values | Meaning |
|---|---|---|
| `resolved` | `false` / `true` | Whether the dispute has been acted on (by the admin or by a completed jury tally) |
| `upheld` | `false` / `true` | `true` if the milestone was found invalid; `false` if it stands |

When a dispute is upheld the admin is expected to revoke or correct the
offending milestone through the standard validator-management flow; the
dispute mechanism itself only records the outcome on-chain.

- Relevant functions: `dispute_milestone`, `resolve_dispute`,
  `cast_dispute_vote`, `tally_dispute`, `set_jury_config`, `get_dispute`,
  `has_dispute` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#verification).
- See [DISPUTE_JURY.md](DISPUTE_JURY.md) for the jury quorum, voting window,
  and conflict-of-interest rules.

---

## Player

A registered footballer with an on-chain identity. A player is identified by a
`player_id` (auto-incremented `u64`) and a Stellar wallet address. Players
start at `ProgressLevel` 0 (Unverified) and advance through up to four levels
as validators approve milestones (Levels 1–2). Level 3 (`EliteTier`) requires
the player to call `confirm_trial_offer` on an Elite-tier scout's logged offer
before its escrow expires; `log_trial_offer` alone does not advance the level.
See [Trial Offer](#trial-offer) and [Progress Level](#progress-level).

- Relevant functions: `register_player`, `get_player`, `filter_players` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#registration).

---

## Progress Level

The four-tier trust ranking attached to every player profile. Levels advance
sequentially; skipping or reversing is blocked by the progress contract (admin
`reset_player_level` is the only exception).

| Level | Variant | Meaning |
|---|---|---|
| 0 | `Unverified` | Profile created, no verifications |
| 1 | `VerifiedIdentity` | Identity confirmed by a validator |
| 2 | `PerformanceMilestones` | Performance stats verified by a validator |
| 3 | `EliteTier` | Player confirmed an Elite-tier scout's trial offer via `confirm_trial_offer` |

- Relevant functions: `advance_level`, `get_level`, `get_progress_history`,
  `reset_player_level` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#progress).

---

## Region Quorum

A region quorum is a geographic-diversity requirement for gated player progress
milestones. It is controlled by `min_region_quorum`, which sets the minimum
number of distinct validator regions that must be represented by the approving
validators before a gated level advancement can commit.

The default value is `0`, which disables the region-quorum requirement. When it
is raised to `2` or more, Level 2 (**Performance Milestones**) requires
approving validators to span at least that many distinct geographic regions,
and the same requirement applies to Level 3 (**Elite Tier**). In other words,
several approvals from validators in the same region do not satisfy a quorum
that requires multiple regions.

The intended purpose is to reduce the risk of validator collusion: a single
validator, or several colluding wallets from the same organization or
a geography, should not be able to push a player through every gated level by
themselves. Requiring geographically distributed validator participation raises
the coordination cost for an attacker and provides a stronger independence
signal for milestone approvals.

> **Design note:** This glossary entry describes the intended design. The
> implementation status of region quorum should be verified against the live
> contract rather than assumed, as the current implementation status is tracked
> separately in the repository issue tracker.

- See [VALIDATOR_COLLUSION_THREAT_MODEL.md](VALIDATOR_COLLUSION_THREAT_MODEL.md)
  for the threat-model rationale.
- Configure the requirement with [`set_min_region_quorum`](CONTRACT_REFERENCE.md#set_min_region_quorum) and inspect it with [`get_min_region_quorum`](CONTRACT_REFERENCE.md#get_min_region_quorum) in [CONTRACT_REFERENCE.md — verification](CONTRACT_REFERENCE.md#verification).

---

## Scout

A talent-discovery professional registered on-chain with a Stellar wallet.
Scouts purchase a subscription tier (`Basic`, `Pro`, or `Elite`) to access the
filtered player pool, pay per-contact fees to unlock player details, and (Elite
only) log trial offers that a player can confirm to reach Level 3.

- Relevant functions: `register_scout`, `subscribe`, `pay_to_contact`,
  `log_trial_offer` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#scout_access).

---

## Stroop

The smallest unit of XLM. 1 XLM = 10 000 000 stroops. All fee fields in
`FeeConfig` and all fee-related return values in the `scout_access` contract
are expressed in stroops (Rust type `i128`).

Worked example: the documented `contact_fee_stroops` value `100000` equals
0.01 XLM (`100000 / 10 000 000`). Keep fee amounts in stroops when comparing
or tuning cost-sensitive calls such as `subscribe`, `pay_to_contact`, and
`batch_contact_players`; their CPU guardrails are tracked in
[`ci/cpu-cost-budget.md`](../ci/cpu-cost-budget.md). If a fee is too low or fee
arithmetic exceeds safe bounds, see the `scout_access` [`InsufficientFee` and
`Overflow` error codes](CONTRACT_REFERENCE.md#scoutaccesserror-scout_access-contract).

---

## Subscription Tier

The access level purchased by a scout. Determines which players are visible and
whether trial offers can be logged.

| Tier | Variant | Notes |
|---|---|---|
| Basic | `Basic` | Access to the filtered player pool |
| Pro | `Pro` | Higher trust signal; wider discovery |
| Elite | `Elite` | Required to call `log_trial_offer` |

Subscriptions expire after `sub_duration_secs` (default 30 days). Downgrades
while a subscription is active are blocked; upgrades charge the full new-tier
fee with no proration.

- Relevant functions: `subscribe`, `get_subscription` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#scout_access).
- Related: [Auto-Renewal](#auto-renewal).

---

## Sybil Resistance

The access-control mechanism that gates Pro-tier subscriptions behind a
verified-scout requirement. `subscribe()` rejects a scout with
`ScoutNotVerified` (`ScoutAccessError` code 27) when the scout is not verified
(or not found), preventing one person from creating many fake scout accounts to
defeat per-scout tier limits.

A scout becomes verified when the platform admin calls `verify_scout(scout_id)`
(marking the profile `verified: true`). The full design rationale — including
the "On-chain verified-tier gating" strategy and the admin verification flow —
is documented in [SYBIL_MITIGATION_DESIGN.md](SYBIL_MITIGATION_DESIGN.md).

- Relevant functions: `verify_scout`, `get_verification`, `subscribe` — see
  [CONTRACT_REFERENCE.md — scout_access](CONTRACT_REFERENCE.md#scout_access).
- Related: [Scout](#scout), [Subscription Tier](#subscription-tier).

---

## Timestamp

All absolute on-chain timestamps in this project are Unix seconds: the number
of seconds elapsed since 1970-01-01 00:00:00 UTC, obtained from the Soroban
ledger timestamp. This applies to fields such as `registered_at`, `updated_at`,
`approved_at`, `disputed_at`, `expires_at`, `subscribed_at`, `contacted_at`,
`logged_at`, and `period_start`, as well as the `since_timestamp` parameter of
`get_history_since`.

`ledger_sequence` is not a timestamp; it is the Soroban ledger sequence number
recorded alongside an event. `sub_duration_secs` is a duration in seconds, not
an absolute Unix timestamp.

Example: a `ProgressEntry` might record `updated_at: 1_735_689_600` and
`ledger_sequence: 12_345_678` for the same level change. The first value is a
Unix-second wall-clock time; the second is the Soroban ledger number that
included the change.

---

## Trial Offer

An escrow-backed, on-chain record that an Elite-tier scout has offered a
player a trial or professional opportunity. [`log_trial_offer`](CONTRACT_REFERENCE.md#log_trial_offerscout-address-player_id-u64-details_hash-string---resultu32-scoutaccesserror)
is step 1: it transfers `trial_offer_escrow_stroops` from the scout, stores a
`TrialOffer` and `TrialEscrow(amount, expires_at)`, and does **not** advance
the player's level by itself.

The player completes step 2 by calling
[`confirm_trial_offer`](CONTRACT_REFERENCE.md#confirm_trial_offerplayer_wallet-address-player_id-u64-index-u32-idempotency_nonce-optionstring---result-scoutaccesserror)
before `expires_at`; a successful confirmation releases the escrow and
advances the player to `EliteTier` (Level 3). If the offer is not confirmed in
time, late confirmation or the admin-only
[`expire_trial_offers`](CONTRACT_REFERENCE.md#expire_trial_offerslimit-u32---resultu32-scoutaccesserror)
sweep refunds the escrowed amount to the originating scout and removes the
pending escrow record.

- See [TRIAL_ESCROW_IMPACT.md](TRIAL_ESCROW_IMPACT.md) and
  [trial-escrow-enumeration.md](trial-escrow-enumeration.md) for the escrow
  rationale and expiry-sweep design.
- Relevant functions: `log_trial_offer`, `confirm_trial_offer`,
  `expire_trial_offers`, `get_trial_offer`, `get_trial_count` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#scout_access).

---

## Validator

A trusted third party (local coach, academy director, or certified trainer)
registered by the platform admin. Only active validators may call
`approve_milestone`. A validator can be revoked by the admin; revoked validators
cannot approve further milestones until re-activated. If a validator is revoked
for cause (e.g. misconduct), their past milestones are flagged so they can be
weighed appropriately by scouts and indexers.

A validator may also carry **specialization tags** (e.g. `"physical-stats"`,
`"identity-kyc"`, `"match-performance"`), set via
`set_validator_specializations`. When `approve_milestone` is called with a
non-`None` `milestone_category`, the contract requires the approving
validator to hold a matching tag, rejecting the call with
`SpecializationMismatch` (code 21) otherwise — preventing, for example, a
pure identity-KYC agent from approving physical performance data. This is
backward-compatible: a validator with no specializations (the default) is
general-purpose, and an **omitted** `milestone_category` remains open to any
active validator regardless of tags.

- Relevant functions: `register_validator`, `revoke_validator`,
  `set_validator_specializations`, `get_validator_status`, `approve_milestone`,
  `get_milestone_with_validator_status` — see
  [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#verification).

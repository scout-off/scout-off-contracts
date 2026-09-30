# Encrypted Evidence and Access Grants

**Status: implemented.** `EvidenceAccessGrant` storage, the query API, and
the `evidence_access_granted` / `evidence_access_revoked` events described
below are live in `contracts/scout_access` (issue #1040).

## Security model

The CID stored in a milestone or trial offer remains a public, immutable
content reference. It proves which payload was approved at a given time, but it
does not provide confidentiality. Privacy comes from making the referenced
payload client-side encrypted before it is uploaded to IPFS or Arweave.

| Property | Mechanism |
|---|---|
| Tamper-proof reference | On-chain `evidence_hash` / `details_hash` CID and approval event |
| Confidential media | Encrypted payload stored at that CID |
| Viewer authorization | `EvidenceAccessGrant` emitted after successful `pay_to_contact` / `batch_contact_players` |

## Key distribution

1. The player client creates a random content-encryption key, encrypts the
   evidence locally, and uploads only ciphertext.
2. The player retains its own key material off-chain. This contract has no
   player-wallet registry, so the player's initial key access is not modeled as
   an on-chain grant.
3. A successful `pay_to_contact` (or `batch_contact_players`, for each newly
   recorded contact) atomically writes an `EvidenceAccessGrant` for
   `(player_id, scout)` and emits `evidence_access_granted`.
4. The frontend/backend watches that event or reads the grant, verifies it, and
   delivers a viewer-specific wrapped key. The viewer decrypts locally.

The smart contracts never receive plaintext media, raw encryption keys, or
wrapped keys. The frontend/backend repository owns encryption, wallet-key
handling, key wrapping, retrieval, and decryption.

## Contract scope

This repository adds the grant storage, query API, and event to the existing
subscription and contact-payment authorization flow. It does not make a public
CID private by itself: uploads must be encrypted before use for the access
grant to have confidentiality value.

### `EvidenceAccessGrant`

```rust
pub struct EvidenceAccessGrant {
    pub player_id: u64,
    pub scout: Address,
    pub granted_at: u64,
    pub tier_at_grant: SubscriptionTier,
    pub revoked: bool,
    pub revoked_at: Option<u64>,
}
```

Written exactly once per successful `(player_id, scout)` contact — the write
is atomic with the contact-fee transfer and the `ContactRecord` write inside
the same `pay_to_contact` / `batch_contact_players` invocation, and is
unreachable on any call that is rejected (insufficient balance, no active
subscription, already contacted, Pro quota exceeded, paused, etc.), because
it runs after every one of those guards has already passed.

### Query API

| Function | Purpose |
|---|---|
| `has_evidence_access(player_id, scout) -> bool` | Fast boolean check for the key-wrapping service: does a non-revoked grant exist? |
| `get_evidence_access_grant(player_id, scout) -> Option<EvidenceAccessGrant>` | Full record, including a revoked grant (to distinguish "never granted" from "granted, then revoked"). |
| `get_player_access_grants(player_id, offset, limit) -> Vec<EvidenceAccessGrant>` | Paginated audit view for a player-facing "who has access to my evidence" UI. `limit` is capped at 50 (`MAX_ACCESS_GRANT_PAGE_LIMIT`, equal to the on-chain page size), so a single call touches at most two fixed-size index pages plus one grant record per returned entry — cost bounded by `limit`, independent of how many grants that player has accumulated in total (proven at 1,000+ grants in `contracts/scout_access/tests/cost_budget.rs`). |

### Grant lifecycle: append-only fact, not a live entitlement

An `EvidenceAccessGrant` records that a scout **was** entitled to contact a
player, and therefore to request that player's evidence key, **at the
moment the grant was issued** — it is not re-derived from the scout's
current subscription state. Concretely:

- **A scout who paid to contact a player while subscribed, and later lets
  the subscription lapse or downgrades, keeps the grant.** The contact and
  the fee already happened; the scout already earned the right to view that
  specific player's evidence. Subscription expiry/downgrade code paths
  (`refund_subscription`, the `subscribe` downgrade guard, `renew_if_due`)
  do not touch `EvidenceAccessGrant` state at all.
- **A scout can never receive a grant while not currently entitled to
  contact**, because the grant write happens only after `pay_to_contact` /
  `batch_contact_players` have already enforced subscription-tier access,
  quota, and payment — there is no separate code path that issues a grant
  independent of a real, paid contact.

**Why append-only-with-explicit-override, instead of auto-revoking a grant
when a subscription lapses:** an auto-revoke-on-downgrade design would
punish scouts for a billing event unrelated to *why* they were granted
access — they paid for a specific player's evidence, not for a
time-boxed subscription to *that grant*. It would also create a race the
frontend/backend would have no clean way to reason about: a wrapped key
already delivered to a scout doesn't become undeliverable just because
their subscription later lapses, so silently flipping the on-chain grant
would create a discrepancy between "the contract says no access" and "the
scout still has the key in hand" — worse than the explicit model below,
which never claims to do something it cannot (see the caveat immediately
below). Compliance/abuse takedowns are handled instead by an explicit admin
action, `admin_revoke_evidence_access(player_id, scout)`, which:

- Is admin-gated (same `require_admin` pattern as `withdraw_fees` /
  `pause_contract`).
- Sets `revoked = true` and `revoked_at`, but **never deletes the grant
  record** — the audit trail of who was ever granted access stays intact
  for `get_player_access_grants`.
- Emits `evidence_access_revoked`.
- Is idempotent: revoking an already-revoked grant is a no-op that returns
  `Ok(())` without re-emitting the event or overwriting `revoked_at`.

> **Caveat — revocation only gates future key-wrap requests.** Per "Contract
> scope" above, the smart contracts never receive plaintext media, raw
> encryption keys, or wrapped keys — key wrapping and delivery are entirely
> the frontend/backend's responsibility. `admin_revoke_evidence_access`
> therefore cannot claw back a wrapped key that the key-wrapping service
> already delivered to a scout before the revoke: it can only instruct that
> service to stop honoring *future* key-wrap requests for this
> `(player_id, scout)` pair (by checking `has_evidence_access` before
> wrapping). A scout who already has the wrapped key retains the ability to
> decrypt the evidence they already fetched; the contract has no mechanism
> to reach into a client that already holds a key. This mirrors the
> equivalent limitation of revoking access to any already-downloaded file.

## Migration

Existing CIDs may already point to unencrypted content and cannot be made
private by adding an on-chain flag. Treat them as legacy public evidence. To
migrate, encrypt the original media, upload the ciphertext under a new CID,
and publish the replacement through an application-level evidence-versioning
workflow; do not overwrite historical approval records.

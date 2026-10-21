# Event Audit Reference

This document lists every on-chain event emitted by the four ScoutChain
contracts, their topic/data schemas, and their indexing obligations. Keep this
file in sync with each contract's `events.rs`.

---

## How events work in Soroban

Each `env.events().publish((topics...), data)` call writes a record to the
transaction's event stream. The Horizon API and Soroban RPC expose these as
`LedgerEntryChange` events under `transaction.result_meta`. Off-chain indexers
(the backend event-stream processor in `scoutchain-backend`) consume these to
keep the Postgres tables in `migrations/001_initial_schema.sql` in sync with
on-chain truth.

---

## Upgrade events

### `contract_upgraded`

Emitted by `upgrade(new_wasm_hash)` in **all four contracts**, *before*
`update_current_contract_wasm` executes, so it is always attributed to the
**old** code version.

| Field  | Value |
|--------|-------|
| Topics | `("contract_upgraded", admin: Address)` |
| Data   | `new_wasm_hash: BytesN<32>` |

**Contracts**: `registration`, `verification`, `progress`, `scout_access`

**Indexer obligation**: Record in `admin_transfers` (or a dedicated
`contract_upgrades` table if added) with the new hash and block timestamp.
Alert on unexpected `contract_upgraded` events in production — an upgrade you
did not initiate is a strong signal of admin key compromise.

**RUNBOOK reference**: See [RUNBOOK.md — Contract Upgrades](RUNBOOK.md#contract-upgrades)
for how to verify the event after running `scripts/upgrade.sh`.

---

## Registration contract events

### `player_registered`
| Field  | Value |
|--------|-------|
| Topics | `("player_registered", wallet: Address)` |
| Data   | `player_id: u64` |

### `scout_registered`
| Field  | Value |
|--------|-------|
| Topics | `("scout_registered", wallet: Address)` |
| Data   | `scout_id: u64` |

### `profile_updated`
| Field  | Value |
|--------|-------|
| Topics | `("profile_updated", wallet: Address)` |
| Data   | `player_id: u64` |

### `scout_verified`
| Field  | Value |
|--------|-------|
| Topics | `("scout_verified", wallet: Address)` |
| Data   | `scout_id: u64` |

### `player_deregistered`
| Field  | Value |
|--------|-------|
| Topics | `("player_deregistered", admin: Address)` |
| Data   | `player_id: u64` |

### `player_deactivated`
| Field  | Value |
|--------|-------|
| Topics | `("player_deactivated", admin: Address)` |
| Data   | `player_id: u64` |

### `player_reactivated`
| Field  | Value |
|--------|-------|
| Topics | `("player_reactivated", admin: Address)` |
| Data   | `player_id: u64` |

### `player_level_synced`
| Field  | Value |
|--------|-------|
| Topics | `("player_level_synced", caller: Address)` |
| Data   | `player_id: u64` |

### `admin_transfer_proposed` / `admin_transferred`
| Field  | Value |
|--------|-------|
| Topics | `("admin_transfer_proposed"/"admin_transferred", old_admin: Address)` |
| Data   | `new_admin: Address` |

---

## Verification contract events

### `milestone_approved`
| Field  | Value |
|--------|-------|
| Topics | `("milestone_approved", validator: Address)` |
| Data   | `(player_id: u64, milestone_index: u32, description: String, evidence_hash: String)` |

### `validator_registered`
| Field  | Value |
|--------|-------|
| Topics | `("validator_registered", wallet: Address)` |
| Data   | `credentials: String` |

### `validator_revoked` / `validator_revoked_for_cause`
| Field  | Value |
|--------|-------|
| Topics | `("validator_revoked"/"validator_revoked_for_cause", admin: Address)` |
| Data   | `(wallet: Address, reason: String)` |

### `dispute_resolved`
| Field  | Value |
|--------|-------|
| Topics | `("dispute_resolved", admin: Address)` |
| Data   | `(player_id: u64, milestone_index: u32, upheld: bool)` |

---

## Progress contract events

### `progress_updated`
| Field  | Value |
|--------|-------|
| Topics | `("progress_updated", updated_by: Address)` |
| Data   | `(player_id: u64, old_level: ProgressLevel, new_level: ProgressLevel)` |

### `player_level_reset`
| Field  | Value |
|--------|-------|
| Topics | `("player_level_reset", admin: Address)` |
| Data   | `(player_id: u64, old_level: ProgressLevel, target_level: ProgressLevel)` |

---

## Scout Access contract events

### `scout_subscribed`
| Field  | Value |
|--------|-------|
| Topics | `("scout_subscribed", scout: Address)` |
| Data   | `(tier: SubscriptionTier, fee_paid: i128)` |

### `player_contacted`
| Field  | Value |
|--------|-------|
| Topics | `("player_contacted", scout: Address)` |
| Data   | `(player_id: u64, fee_paid: i128)` |

### `trial_offer_logged`
| Field  | Value |
|--------|-------|
| Topics | `("trial_offer_logged", scout: Address)` |
| Data   | `player_id: u64` |

### `trial_offer_confirmed`
| Field  | Value |
|--------|-------|
| Topics | `("trial_offer_confirmed", scout: Address)` |
| Data   | `(player_id: u64, index: u32)` |

### `fees_withdrawn`
| Field  | Value |
|--------|-------|
| Topics | `("fees_withdrawn", admin: Address)` |
| Data   | `(to: Address, amount: i128, timestamp: u64)` |

---

## Diagnostic-only events (not committed to ledger)

These events appear only in the **diagnostic stream** — they are emitted
immediately before a transaction-aborting error, so they do not appear in
committed ledger state. Indexers can detect them in raw transaction metadata.

| Event | Contract | Trigger |
|-------|----------|---------|
| `progress_call_failed` | verification, scout_access | Cross-contract `advance_level` fails |
| `progress_contract_not_set` | verification, scout_access | Progress contract not wired |
| `level_advancement_skipped` | verification | Player already at EliteTier |

---

*Last updated: 2026-09-27 — added `contract_upgraded` event (issue #1434).*

# Event History Audit Tool

`scripts/audit-event-history.js` reconstructs a player's complete derived state purely from raw blockchain events and validates the reconstruction against live contract state and the indexer database.

It detects a class of bug that `scripts/reconcile-indexer.js` cannot: internal inconsistencies in the event chain itself (e.g., a level transition that violates the documented 0→1→2→3 rule, or milestone events that reference non-existent milestones).

---

## When to Use This vs. `reconcile-indexer.js`

| Tool | When to use | Strengths | Limitations |
|------|-----------|-----------|------------|
| **reconcile-indexer.js** | Regular scheduled audits (hourly/daily) against production | Fast, already integrated | Assumes indexer replay logic is correct; can't detect event-chain bugs |
| **audit-event-history.js** | After a suspected data corruption or "impossible state" in a player record | Detects internal event inconsistencies | Slower (replays full event history); requires RPC access to event stream |

**Together:** Run `reconcile-indexer.js` for ongoing drift detection. When it flags a mismatch you can't explain, use `audit-event-history.js` to dig into whether the event chain itself is self-consistent.

---

## How It Works

1. **Fetch all events** from each contract (progress, verification, scout_access) directly from the Soroban RPC event stream, from genesis to now.
2. **Sort by ledger sequence** to ensure chronological order.
3. **Replay the event log** for a specific player, reconstructing:
   - Level progression (each `progress_updated` and `player_level_reset` event)
   - Milestones (each `milestone_approved` event)
   - Disputes (each `milestone_disputed` / `dispute_resolved`)
   - Trial offers (each `trial_offer_logged` / `trial_offer_confirmed` / `trial_offer_expired`)
4. **Validate event-chain consistency** independent of any current state:
   - Each `progress_updated` event's `old_level` must match the current tracked level
   - Level transitions must follow the 0→1→2→3 progression (or explicit resets)
   - Milestone/dispute/trial events must reference valid player/milestone indices
5. **Compare the reconstruction** against:
   - Live contract state (via `stellar contract invoke get_level`, etc.)
   - Indexer database (via Postgres query)
6. **Report findings**:
   - Internal event-chain inconsistencies (errors)
   - Mismatches with live state (separate category)
   - Mismatches with indexer (separate category)

---

## Installation & Setup

Requires:

- Node.js >= 18 and `npm install` run once at the repo root.
- The [pinned `stellar-cli` version](CONTRIBUTING.md#installing-the-pinned-stellar-cli-version).
- RPC access to a Soroban network (testnet, mainnet, or local sandbox).
- Optionally: `DATABASE_URL` for Postgres comparison.

The four `*_CONTRACT_ID` variables can be sourced from `.env.contracts` (written by `deploy.sh`) or exported directly.

---

## Usage

### Audit a single player

```bash
RPC_URL=https://soroban-testnet-rpc.stellar.org \
PROGRESS_CONTRACT_ID=C... \
VERIFICATION_CONTRACT_ID=C... \
SCOUT_ACCESS_CONTRACT_ID=C... \
  node scripts/audit-event-history.js 42
```

### Audit all players (sample first 10)

```bash
RPC_URL=https://soroban-testnet-rpc.stellar.org \
PROGRESS_CONTRACT_ID=C... \
  node scripts/audit-event-history.js all --sample 10
```

### Include indexer comparison

```bash
RPC_URL=https://soroban-testnet-rpc.stellar.org \
DATABASE_URL=postgres://user:pass@host:5432/scoutchain \
PROGRESS_CONTRACT_ID=C... \
  node scripts/audit-event-history.js 42
```

### Output as JSON

```bash
... node scripts/audit-event-history.js 42 --json > audit-report.json
```

---

## Output Interpretation

### Event-Chain Consistency Issues

If the tool reports an event-chain error (e.g., "progress_updated oldLevel 2 doesn't match current 1"), the event history itself is corrupt — this is a hard data integrity bug, either in:

- The contract logic that emitted the event (wrong old_level recorded)
- A ledger reorg that partially rolled back some events but not others
- An indexer that processed events out of chronological order

**Action:** Escalate to the contract team. This is not a reconciliation mismatch — it's proof the on-chain history is inconsistent.

### Mismatch with Live Contract State

If the tool's reconstructed level doesn't match what `progress.get_level` returns, either:

- The current contract state is inconsistent with its own event history (a bug in the contract or a state-resetting incident)
- The event stream is incomplete (events were not persisted, or the RPC is missing some)

**Action:** Check whether the RPC's event stream is complete. Compare timestamps with known incidents. If events are legitimately missing, investigate whether an upgrade or admin reset operation cleared them.

### Mismatch with Indexer

If the reconstructed level doesn't match the indexer's copy, but does match live state:

- The indexer replayed events incorrectly, or
- The indexer is behind (hasn't processed all events yet)

**Action:** Check the indexer's `indexer_cursor` ledger position. If it's behind, wait for it to catch up. Otherwise, refer to [INDEXER.md](INDEXER.md) for drift resolution.

---

## Event Types Audited

The tool reconstructs state from these events:

| Event | Contract | Reconstructs | Chain-validation |
|-------|----------|--------------|------------------|
| `progress_updated` | progress | Level progression | `old_level` must match current; transition must follow rules |
| `player_level_reset` | progress | Level reset | Must be valid reset-to level |
| `milestone_approved` | verification | Milestone registry | Milestone must be unique per (player, index) |
| `milestone_disputed` | verification | Dispute state (including impact_score, jury_required, quorum, voting_deadline) | Milestone must exist |
| `dispute_vote_cast` | verification | Per-validator jury vote | Dispute must be jury-required and unresolved; validator must be eligible |
| `dispute_tallied` | verification | Jury dispute outcome | Dispute must be jury-required; votes_for+votes_against must match accumulated cast events |
| `dispute_resolved` | verification | Admin dispute outcome | Dispute must be non-jury and in "disputed" state |
| `trial_offer_logged` | scout_access | Trial offer creation | Trial must be unique per (player, index) |
| `trial_offer_confirmed` | scout_access | Trial state transition | Must follow logged → confirmed or expired path |
| `trial_offer_expired` | scout_access | Trial state transition | Must follow logged → expired or confirmed path |
| `milestone_flagged` | verification | Milestone pending-re-review flag | Milestone must exist; validator must be revoked for cause |
| `milestone_flag_cleared` | verification | Flag clearance | Flag must be set; reviewer must be an active validator |
| `revocation_cascade_complete` | verification | End of cascade sweep | Emitted after all milestones for a for-cause revocation are flagged |
| `revocation_cascade_continued` | verification | Partial cascade sweep | Cursor stored; `continue_revocation_cascade` required to finish |

---

## Performance Considerations

- **Speed:** Slower than `reconcile-indexer.js` because it replays the entire event history from genesis. For a player with 100+ events, expect a few seconds per player.
- **Memory:** Holds all events in memory. For a network with millions of events, consider pagination or single-contract filtering.
- **RPC cost:** Fetches the full event stream, which is expensive on public RPCs. For production use, cache events locally or use a full-history node.

To speed up audits:

- Use `--sample N` to limit the number of players when running "all" mode.
- Filter to specific players of interest rather than auditing all at once.
- Cache events locally between runs.

---

## Known Limitations

1. **Cannot detect missing events** from the RPC's perspective (only from the contract's perspective). If the RPC itself doesn't have an event, this tool can't know.
2. **Does not validate external data** (e.g., whether an evidence hash is a valid IPFS CID). Only validates the event chain itself.
3. **Requires database access** for indexer comparison. Without `DATABASE_URL`, only validates internal consistency and live state.
4. **Does not audit fee_config_history, admin_transfers, or validator_history** — these are pure event logs with no "current state" to reconstruct (they are audited separately, see [INDEXER.md](INDEXER.md)).
5. **Does not replay `evidence_access_granted` / `evidence_access_revoked`** — `scripts/reconcile-indexer.js` reconciles the `evidence_access_grants` table directly against `scout_access.get_player_access_grants` (a current-state getter, not an event replay), which is sufficient for that data shape; see [INDEXER.md](INDEXER.md) and [EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md). Event-chain validation here (e.g. "no `evidence_access_revoked` without a prior `evidence_access_granted` for the same pair") is a reasonable future extension of this tool but is not implemented yet.
6. **Does not audit the k-of-n attestation flow** — `attestation_recorded`, `attestation_window_expired`, and `validator_votes_invalidated` are not currently replayed or chain-validated by this tool. Pending vote tallies, per-validator vote status, and round resets are therefore not cross-validated against on-chain state by `audit-event-history.js` today.

---

## Integration with CI/CD

For production use, integrate into an automated reconciliation pipeline:

```bash
#!/bin/bash
set -e

RPC_URL=https://soroban-mainnet-rpc.stellar.org
DATABASE_URL=postgres://...

# Sample 100 random players for a quick audit
SAMPLE_SIZE=100

echo "Running event-history audit..."
node scripts/audit-event-history.js all --sample $SAMPLE_SIZE --json > audit-report.json

# Parse the JSON and alert if any consistency errors found
if jq '.[] | select(.reconstructedState.issues[] | select(.severity=="error"))' audit-report.json | grep -q .; then
  echo "ALERT: Event-chain consistency errors detected"
  cat audit-report.json | jq '.[] | select(.reconstructedState.issues[])'
  exit 1
fi

echo "✓ Audit passed"
```

---

## Troubleshooting

### RPC timeout or rate-limit

**Error:** `RPC error: request timed out` or similar.

**Cause:** The RPC is slow or rate-limiting event fetches.

**Solution:** 
- Use a private or full-history node instead of a public RPC
- Reduce `--sample` size to audit fewer players
- Cache events locally and filter in-process

### Events not available

**Error:** `getEvents returned 0 events`

**Cause:** The RPC doesn't have a complete event history (common on public RPCs, which may only store recent events).

**Solution:**
- Use a full-history node (self-hosted or Fishery)
- Reduce scope to recent ledgers

### Database connection failed

**Error:** `Failed to query indexer: connection refused`

**Cause:** `DATABASE_URL` is invalid or the database is offline.

**Solution:**
- Omit `--DATABASE_URL` to skip indexer comparison
- Or, fix the connection string and retry

---

## Related Documentation

- [INDEXER.md](INDEXER.md) — Reconciliation against the Postgres mirror; when to run each tool
- [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md) — Full reference for contract getters
- [RUNBOOK.md](RUNBOOK.md) — Emergency procedures and operational tasks


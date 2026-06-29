# ScoutChain — AI Integration Guide

This document is the authoritative reference for AI assistants, SDK consumers, and new team members integrating with ScoutChain's Soroban smart contracts.

---

## Contract Overview

| Contract | Package | Purpose |
|----------|---------|---------|
| `registration` | `scoutchain-registration` | Player & scout on-chain identity |
| `verification` | `scoutchain-verification` | Validator registry & milestone approvals |
| `progress` | `scoutchain-progress` | Four-tier level state machine |
| `scout_access` | `scoutchain-scout-access` | Subscriptions, pay-to-contact, trial offers |

---

## Shared Types

All four contracts import from `scoutchain-shared-types`:

```rust
pub enum ProgressLevel {
    Unverified,          // Level 0
    VerifiedIdentity,    // Level 1
    PerformanceMilestones, // Level 2
    EliteTier,           // Level 3
}
```

---

## Cross-Contract Wiring

`approve_milestone` (verification contract) cross-calls `advance_level` (progress contract) atomically — both state changes occur in the same Stellar transaction. This wiring must be established after every fresh deployment via `initialize.sh` or manually. See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for the full wiring procedure.

`log_trial_offer` (scout_access contract) also cross-calls `advance_level` on the progress contract to advance a player to Level 3.

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `REGISTRATION_CONTRACT_ID` | Deployed registration contract ID |
| `VERIFICATION_CONTRACT_ID` | Deployed verification contract ID |
| `PROGRESS_CONTRACT_ID` | Deployed progress contract ID |
| `SCOUT_ACCESS_CONTRACT_ID` | Deployed scout_access contract ID |

---

## TypeScript Bindings

After deployment run `./scripts/generate-bindings.sh <network>`. Import the generated clients:

```typescript
import { Client as RegistrationClient } from "@scoutchain/bindings-registration";
import { Client as VerificationClient }  from "@scoutchain/bindings-verification";
import { Client as ProgressClient }      from "@scoutchain/bindings-progress";
import { Client as ScoutAccessClient }   from "@scoutchain/bindings-scout-access";
```

See `bindings/README.md` for full usage details.

---

## Error Handling

### `ProgressCallFailed`

#### What it is

`ProgressCallFailed` is returned by two contracts when the cross-contract call to `progress.advance_level` fails at runtime:

| Contract | Error enum | Code |
|----------|-----------|------|
| `verification` | `VerificationError::ProgressCallFailed` | 12 |
| `scout_access` | `ScoutAccessError::ProgressCallFailed` | 14 |

#### When it is returned

- **`verification` contract** — inside `approve_milestone`, after the milestone record is written to storage, the contract calls `progress.advance_level`. If that call fails for any reason (contract not deployed, not wired, panics, or returns an unexpected error), `ProgressCallFailed` is returned.
- **`scout_access` contract** — inside `log_trial_offer`, after the trial offer is recorded, the contract calls `progress.advance_level` to advance the player to Level 3. A failure in that call returns `ProgressCallFailed`.

#### Cause and effect

> **Key invariant:** The milestone or trial offer record is **already committed** to storage before the cross-contract call. If `ProgressCallFailed` is returned, the on-chain record exists but the player's progress level was **not** advanced.

This split state occurs because Soroban does not automatically roll back storage writes on a sub-call failure unless the entire transaction is reverted. The milestone write and the cross-contract call are in the same transaction, so a `ProgressCallFailed` error aborts the entire transaction — no partial state is committed. The player's level is not advanced and the milestone is not recorded.

In short: `ProgressCallFailed` means the entire `approve_milestone` or `log_trial_offer` transaction was reverted.

#### Recommended recovery steps for SDK consumers

1. **Re-query the player level** immediately after receiving `ProgressCallFailed`:
   ```typescript
   const player = await progressClient.getPlayer({ player_id });
   console.log("Current level:", player.level);
   ```
   If the level did not advance, the root cause is a wiring problem, not a data issue.

2. **Check if the progress contract is wired** — `ProgressCallFailed` almost always means the verification or scout_access contract does not have the progress contract address registered. Confirm by calling:
   ```bash
   stellar contract invoke --id $VERIFICATION_CONTRACT_ID -- get_progress_contract
   ```
   An empty or missing result confirms the wiring is absent.

3. **Re-wire the contracts** — run `initialize.sh` again or apply the manual wiring commands documented in [docs/DEPLOYMENT.md — Cross-Contract Wiring](docs/DEPLOYMENT.md#common-mistakes):
   ```bash
   ./scripts/initialize.sh testnet
   ```

4. **Alert the admin** if the error is systematic (every `approve_milestone` call fails) — this indicates the progress contract was never wired or was re-deployed without re-running `initialize.sh`.

5. **Retry the transaction** — once wiring is confirmed, the original `approve_milestone` or `log_trial_offer` call can be retried safely. Because the full transaction was reverted, there is no duplicate-write risk.

#### Summary

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Every `approve_milestone` returns `ProgressCallFailed` | Progress contract not wired after deployment | Run `./scripts/initialize.sh` |
| Intermittent `ProgressCallFailed` | Progress contract re-deployed without re-wiring | Re-wire with `set_progress_contract` |
| `ProgressCallFailed` on `log_trial_offer` | Scout_access → progress link missing | Re-run `initialize.sh` or wire manually |

For the full wiring procedure, see [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

---

## Events Reference

| Event topic | Emitting contract | Payload fields |
|-------------|------------------|---------------|
| `player_registered` | registration | `player_id`, `wallet` |
| `milestone_approved` | verification | `player_id`, `milestone`, `validator` |
| `progress_updated` | progress | `player_id`, `old_level`, `new_level` |
| `scout_subscribed` | scout_access | `scout`, `tier`, `expires_at` |
| `player_contacted` | scout_access | `scout`, `player_id` |
| `trial_offer_logged` | scout_access | `scout`, `player_id`, `details_hash` |
| `fees_withdrawn` | scout_access | `to`, `amount` |

---

## Common Integration Pitfalls

- **Wiring must be re-run after every fresh deployment.** Contract IDs change on each deploy; the old wiring references stale IDs.
- **`initialize` is one-time per contract.** Calling it twice returns `AlreadyInitialized` (code 1). This is not an error — the contract is already ready.
- **Error codes are per-contract, not global.** Code `4` means `Unauthorized` in the verification contract but `Unauthorized` (different context) in scout_access. Always check which contract returned the error.
- **Subscription tier check is enforced on-chain.** Basic scouts cannot call `pay_to_contact` without an active subscription. Elite is required for `log_trial_offer`.

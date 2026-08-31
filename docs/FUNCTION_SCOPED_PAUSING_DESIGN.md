# Function-Scoped Pausing: Reference Implementation for `approve_milestone` (#809)

## Overview

This document describes a general pattern for function-scoped pausing (independent kill switches for individual contract functions) using `approve_milestone` in the verification contract as the concrete reference implementation.

Today's circuit breaker (`pause_contract`) is all-or-nothing: it halts every state-changing function simultaneously. This pattern enables pausing a specific high-risk function without disrupting read queries or other operations.

---

## Rationale for Starting with `approve_milestone`

From CODEOWNERS:
> `/contracts/verification/` - Validator authorization is a platform trust anchor

`approve_milestone` is the highest-trust function in the verification contract:
- Called by validators to record player milestones
- Triggers cross-contract calls to the progress contract to advance player levels
- Any collusion or compromise by validators directly impacts player reputation
- Decoupling it from the whole-contract pause allows incident response without halting validator registration, revocation, or status queries

---

## Pattern Design

### 1. Naming Convention

For a function `{function_name}`, implement:
- **Storage Flag:** `Paused{FunctionName}` (e.g., `PausedApproveMilestone` for `approve_milestone`)
- **Pause Function:** `pause_{function_name}()` (e.g., `pause_approve_milestone`)
- **Unpause Function:** `unpause_{function_name}()` (e.g., `unpause_approve_milestone`)
- **Check Helper:** `require_{function_name}_not_paused()` (e.g., `require_approve_milestone_not_paused`)
- **Events:** `{function_name}_paused` / `{function_name}_unpaused` (e.g., `approve_milestone_paused` / `approve_milestone_unpaused`)
- **Error Code:** `{FunctionName}Paused` (e.g., `ApproveMilestonePaused`)

### 2. Storage Key Convention

```rust
#[contracttype]
pub enum DataKey {
    // ... existing keys ...
    
    // Circuit breaker for approve_milestone (independent of whole-contract Paused flag)
    PausedApproveMilestone,
}
```

Stored in **instance storage** (not persistent) because:
- Pause state is operational/administrative, not user-facing or identity-critical
- Instance TTL is short (100-10000 ledgers); pause state changes frequently
- No risk of archival (instance storage is not subject to TTL)
- Faster access in hot-path functions

### 3. Implementation Pattern

#### Admin Functions

```rust
pub fn pause_approve_milestone(env: Env) -> Result<(), VerificationError> {
    let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
    env.storage().instance().set(&DataKey::PausedApproveMilestone, &true);
    events::approve_milestone_paused(&env, &admin);
    Ok(())
}

pub fn unpause_approve_milestone(env: Env) -> Result<(), VerificationError> {
    let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
    env.storage().instance().set(&DataKey::PausedApproveMilestone, &false);
    events::approve_milestone_unpaused(&env, &admin);
    Ok(())
}
```

#### Check Helper

```rust
fn require_approve_milestone_not_paused(env: &Env) -> Result<(), VerificationError> {
    let paused = env.storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::PausedApproveMilestone)
        .unwrap_or(false);
    if paused {
        return Err(VerificationError::ApproveMilestonePaused);
    }
    Ok(())
}
```

#### In the Target Function

```rust
pub fn approve_milestone(
    env: Env,
    validator_wallet: Address,
    player_id: u64,
    description: String,
    evidence_hash: String,
) -> Result<u32, VerificationError> {
    // Check BOTH whole-contract pause AND function-scoped pause
    Self::require_not_paused(&env)?;  // Existing whole-contract pause
    Self::require_approve_milestone_not_paused(&env)?;  // New function-scoped pause
    
    // ... rest of function ...
}
```

### 4. Events

```rust
// In events.rs

pub const APPROVE_MILESTONE_PAUSED: &str = "approve_milestone_paused";
pub const APPROVE_MILESTONE_UNPAUSED: &str = "approve_milestone_unpaused";

/// topics: (event_name, admin)  data: ()
pub fn approve_milestone_paused(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "approve_milestone_paused"), admin.clone()),
        (),
    );
}

/// topics: (event_name, admin)  data: ()
pub fn approve_milestone_unpaused(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "approve_milestone_unpaused"), admin.clone()),
        (),
    );
}
```

### 5. Error Code

```rust
// In errors.rs

#[contracterror]
pub enum VerificationError {
    // ... existing errors ...
    
    /// The approve_milestone function is paused independently.
    ApproveMilestonePaused = 20,
}
```

### 6. Health Query Update

Extend the health response to include function-scoped pause states:

```rust
#[contracttype]
#[derive(Clone, Debug)]
pub struct ContractHealth {
    pub initialized: bool,
    pub paused: bool,
    pub approve_milestone_paused: bool,
}

pub fn health(env: Env) -> ContractHealth {
    let initialized = env.storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::Initialized)
        .unwrap_or(false);
    let paused = env.storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::Paused)
        .unwrap_or(false);
    let approve_milestone_paused = env.storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::PausedApproveMilestone)
        .unwrap_or(false);

    ContractHealth {
        initialized,
        paused,
        approve_milestone_paused,
    }
}
```

---

## Impact Analysis

### Functions Affected

- **approve_milestone**: Now checks both whole-contract `Paused` and function-scoped `PausedApproveMilestone`
- **register_validator**, **revoke_validator**, **batch_register_validators**, etc.: **Unchanged** (still only check whole-contract `Paused`)
- **All read functions**: **Unchanged** (never check any pause flag)

### Pause Semantics

| Scenario | Whole-Contract Pause | approve_milestone Pause | Outcome |
|----------|----------------------|------------------------|---------|
| Both off | ✓ | ✓ | approve_milestone works; all other functions work |
| Whole-contract on | ✗ | ✓ | Everything halted (whole-contract takes precedence) |
| Whole-contract off | ✓ | ✗ | approve_milestone halted; other functions work |
| Both on | ✗ | ✗ | Everything halted |

### Operational Use Cases

**When to use `pause_approve_milestone` instead of `pause_contract`:**
1. **Validator collusion detected**: Pause milestone approval while validators are investigated; continue registering new validators and processing renewals.
2. **Cross-contract issue**: If the progress contract has a bug, pause `approve_milestone` without blocking validator admin operations.
3. **Partial incident**: Freeze milestone recording while other contract logic remains operational for monitoring and testing.

**When to use `pause_contract`:**
1. Critical vulnerability in the contract itself
2. Need to halt all state-changing operations immediately

---

## Second Implementation: `pay_to_contact` (scout_access, #1056)

The pattern was next applied to `pay_to_contact` in the `scout_access` contract —
its second real use. It follows the conventions above exactly:

- **Storage Flag:** `DataKey::PausedPayToContact` (instance storage)
- **Pause / Unpause:** `pause_pay_to_contact()` / `unpause_pay_to_contact()`
- **Check Helper:** `require_pay_to_contact_not_paused()`, called from `pay_to_contact()`
- **Events:** `pay_to_contact_paused` / `pay_to_contact_unpaused`
- **Error Code:** `ScoutAccessError::PayToContactPaused = 36`
- **Health Query:** `pay_to_contact_paused: bool` added to the scout_access health response

Note the error code is `36`, not a low number: each contract's error enum is
independent, so a function-scoped pause code just takes the next free slot in
that contract's `errors.rs`.

---

## Reusability for Future Functions

To apply this pattern to another function (e.g., `resolve_dispute` in the
verification contract):

1. Add new `DataKey` variant: `PausedResolveDispute`
2. Add new error code in that contract's `errors.rs` at the next free slot
   (in `verification` that is currently `ResolveDisputePaused = 37` — codes
   0–36 are already taken; always check `errors.rs` before assigning)
3. Add new events: `resolve_dispute_paused` / `resolve_dispute_unpaused`
4. Implement `pause_resolve_dispute()` and `unpause_resolve_dispute()` functions
5. Add `require_resolve_dispute_not_paused()` helper
6. Insert check in `resolve_dispute()`: `Self::require_resolve_dispute_not_paused(&env)?;`
7. Update `health()` to include `resolve_dispute_paused: bool`
8. Add tests

Total effort: ~30 lines of code per additional function. No re-derivation needed; just follow the pattern.

---

## Testing Strategy

### Unit Tests

1. **test_approve_milestone_blocked_by_whole_contract_pause**
   - Pause the entire contract, verify approve_milestone returns error
   
2. **test_approve_milestone_blocked_by_function_pause**
   - Pause only approve_milestone, verify it returns `ApproveMilestonePaused` error
   
3. **test_approve_milestone_succeeds_when_unpause**
   - Pause, then unpause, verify approve_milestone works
   
4. **test_other_functions_not_affected_by_function_pause**
   - Pause approve_milestone, verify register_validator, revoke_validator, etc. still work
   
5. **test_pause_approve_milestone_requires_admin**
   - Non-admin attempts pause_approve_milestone, verify rejection
   
6. **test_health_includes_function_pause_state**
   - Query health(), verify `approve_milestone_paused` field is present and accurate

### Integration Tests

- Full incident scenario: Pause approve_milestone, continue validator operations, then unpause

---

## Documentation: RUNBOOK Update

Add a new section to `docs/RUNBOOK.md`:

```markdown
## Function-Scoped Circuit Breakers

### When to Use `pause_approve_milestone` Instead of `pause_contract`

**Use `pause_approve_milestone` if:**
- Only milestone approval has been compromised or has a bug
- Validators are being investigated; don't block validator registration/revocation
- Cross-contract issue with progress contract; other verification logic is fine

**Use `pause_contract` (whole contract) if:**
- Multiple functions are affected
- Vulnerability is in core contract logic, not a specific function
- Need immediate shutdown of all state changes

### Example: Validator Collusion Incident

```bash
# 1. Detect suspicious milestones from specific validator
# 2. Pause only approve_milestone while investigation continues
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- pause_approve_milestone

# 3. Continue validator operations (registration, revocation)
# 4. Query health to confirm state
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- health

# 5. Once investigation complete, unpause
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- unpause_approve_milestone
```

### Monitoring

Subscribe to events:
- `approve_milestone_paused` — Function-scoped pause activated
- `approve_milestone_unpaused` — Function-scoped pause lifted
- `contract_paused` — Whole-contract pause (overrides function-scoped state)
```

---

## Backwards Compatibility

- Existing contracts using only whole-contract `pause_contract` are unaffected
- `approve_milestone` now checks an additional flag, but defaults to `false` (unpause state)
- No breaking changes to API or error codes
- Health response adds optional field (if not provided, assume `false`)

---

## Future Extensions

1. **Automatic un-pause after delay**: Implement time-based un-pause for lesser-critical functions
2. **Per-validator pause**: Only pause milestones from specific validators (more granular)
3. **Incident logging**: Record pause/unpause events with external timestamp for audit trail
4. **Cross-contract coordination**: If multiple contracts adopt this pattern, coordinate pause states via shared registry

---

## Summary

This pattern provides:
- ✅ Clear naming convention for reusability
- ✅ Independent kill switch for high-trust functions
- ✅ Backwards compatible (no breaking changes)
- ✅ Observable via events and health queries
- ✅ Documented runbook section for operational use
- ✅ General-purpose design for adoption on second and subsequent functions

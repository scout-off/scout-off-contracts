# ScoutChain Runbook

Operational procedures for the ScoutChain platform.

---

## Emergency: Pause All Contracts

Use this procedure when a security incident requires immediately halting all
state-changing contract operations (e.g. a critical bug is being actively
exploited).

### Prerequisites

- `ADMIN_SECRET` — the Stellar secret key for the platform admin account.
- `SOROBAN_RPC_URL` and `STELLAR_NETWORK` set in your environment (or `.env`).
- All four contract IDs available in `.env.contracts`.

```bash
# Load environment variables
source .env
source .env.contracts
```

### One-command pause script

Run the emergency pause script:

```bash
./scripts/emergency-pause.sh
```

> **Note**: This requires `ADMIN_SECRET` and the contract IDs in `.env.contracts`
> to be available first (see the prerequisites above).
> Each `pause_contract` call is a separate Stellar transaction.
> If the script exits mid-way (e.g. network error), run it again — the already-
> paused contracts will return `ContractPaused` but will not change state.
> Continue from the failed contract manually if needed.

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
# 1. Pause only approve_milestone while investigation continues
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- pause_approve_milestone

# 2. Continue validator operations (registration, revocation)
# 3. Query health to confirm state
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- health

# 4. Once investigation complete, unpause
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- unpause_approve_milestone
```

### When to Use `pause_pay_to_contact` Instead of `pause_contract`

**Use `pause_pay_to_contact` if:**
- Fee-charging `pay_to_contact` has been compromised or has a bug
- Need to halt contact fees while keeping scout operations (subscribe, renew, read state) running
- Cross-contract issue affecting payments but not scout admin functions

**Use `pause_contract` (whole contract) if:**
- Multiple functions are affected
- Core contract logic vulnerability
- Need immediate shutdown of all state changes

### Example: Payment Issue Incident

```bash
# 1. Pause only pay_to_contact while investigation continues
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- pause_pay_to_contact

# 2. Continue scout operations (subscribe, renew, read state)
# 3. Query health to confirm state
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- health

# 4. Once investigation complete, unpause
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- unpause_pay_to_contact
```

### Monitoring

Subscribe to events to detect and verify pause state changes:

- `approve_milestone_paused` — Function-scoped pause for verify activated
- `approve_milestone_unpaused` — Function-scoped pause for verify lifted
- `pay_to_contact_paused` — Function-scoped pause for scout_access activated
- `pay_to_contact_unpaused` — Function-scoped pause for scout_access lifted
- `contract_paused` — Whole-contract pause (overrides function-scoped state)
```

### Manual pause (contract by contract)

If you prefer to pause contracts individually:

```bash
source .env && source .env.contracts
NETWORK_ARGS="--network $STELLAR_NETWORK --source $ADMIN_SECRET"

stellar contract invoke --id "$REGISTRATION_CONTRACT_ID" $NETWORK_ARGS -- pause_contract
stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" $NETWORK_ARGS -- pause_contract
stellar contract invoke --id "$PROGRESS_CONTRACT_ID"     $NETWORK_ARGS -- pause_contract
stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" $NETWORK_ARGS -- pause_contract
```

### Verify each contract is paused

After pausing, confirm `health().paused == true` for all four contracts:

```bash
source .env && source .env.contracts
NETWORK_ARGS="--network $STELLAR_NETWORK --source $ADMIN_SECRET"

echo "registration:" && stellar contract invoke --id "$REGISTRATION_CONTRACT_ID" $NETWORK_ARGS -- health
echo "verification:" && stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" $NETWORK_ARGS -- health
echo "progress:"     && stellar contract invoke --id "$PROGRESS_CONTRACT_ID"     $NETWORK_ARGS -- health
echo "scout_access:" && stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" $NETWORK_ARGS -- health
```

Expected output for each contract:

```json
{"initialized":true,"paused":true}
```

A `"paused":false` response means that contract was not successfully paused —
re-run the pause command for that contract before proceeding.

---

## Emergency: Admin Key Loss / Compromise

Every admin-gated function in every contract — `pause_contract`,
`upgrade`, `propose_admin`/`accept_admin`, `set_progress_contract`,
`update_fee_config`, `revoke_validator`, and so on — requires
`require_auth()` from the single address stored as `Admin`. There is no
on-chain fallback if that key is unrecoverable: Soroban has no
social-recovery or governance primitive to fall back to, and the two-step
`propose_admin` / `accept_admin` rotation still requires the *current*
admin to sign the first step. This is the scenario one level worse than
"we need to pause a buggy contract" (see the pause procedure above): here,
the team cannot even issue that first `pause_contract` call.

For response-time expectations and incident-severity guidance, see
[`SECURITY.md#emergency-response-immediate-mitigation`](../SECURITY.md#emergency-response-immediate-mitigation).

This section assumes the multisig/timelock admin work tracked in
[issue #609](https://github.com/scout-off/scout-off-contracts/issues/609)
has not shipped yet — once it has, this single-key failure mode mostly goes
away, which is exactly why that issue should be prioritized (see "Prevention"
below).

### Step 1 — Decide which scenario you're in

This determines everything else, so establish it first:

| Signal | Scenario |
|---|---|
| The secret key/hardware wallet is destroyed or was never backed up; no unauthorized transactions have occurred | **Lost** — no attacker |
| Unexpected admin transactions appear (validator revocations, fee changes, an `upgrade()` you didn't initiate, an unfamiliar `propose_admin`) | **Compromised** — active attacker |
| You're not sure | Treat it as **Compromised** until proven otherwise — the cost of over-reacting (an unnecessary migration) is far lower than the cost of under-reacting to an active attacker |

### Scenario A — Key merely lost (no attacker)

No one can act maliciously, but no one can act at all either: the admin
address is frozen exactly as it was at the moment of loss. Concretely:

- If the contract was **unpaused** when the key was lost, it keeps running
  normally forever, just without any admin lever (no more validator
  changes, fee updates, or upgrades — ever, on this contract instance).
- If the contract was **paused** when the key was lost, it is now
  **permanently paused** — this is the worst version of this scenario,
  since the contract is fully inert with no path back.

There is no urgency to warn users publicly (nothing is actively being
misused), but the platform has permanently lost the ability to operate that
contract. The only way out is the address-migration procedure in Scenario B
— run it on your own timeline rather than under incident pressure.

### Scenario B — Key compromised (active attacker)

The attacker now holds sole admin rights. They can revoke every validator,
rewrite fee config, propose (and, from a second address they also control,
accept) a new admin, or push a malicious `upgrade()` — and the legitimate
team **cannot call `pause_contract` to stop any of it**, because that also
requires the now-compromised admin key.

Since the contract itself cannot be locked down, mitigation has to happen
around it:

1. **Public announcement, immediately.** Tell users the admin key for
   contract ID `$COMPROMISED_ID` is compromised and the contract should be
   considered abandoned as of now. Include the exact contract ID — this is
   the only thing that unambiguously identifies which deployment is unsafe.
2. **Client-side blocklisting.** The frontend and backend (both outside
   this repo) must stop reading from and writing to the compromised
   contract ID immediately — this is the only real "circuit breaker" left
   once `pause_contract` is unavailable. Coordination points to hand to
   those teams:
   - The exact compromised contract ID(s) (`REGISTRATION_CONTRACT_ID` /
     `VERIFICATION_CONTRACT_ID` / `PROGRESS_CONTRACT_ID` /
     `SCOUT_ACCESS_CONTRACT_ID` — whichever is affected; a compromised
     `progress` admin key is the scenario this issue was filed against,
     but the same procedure applies to any of the four).
   - A hard denylist entry so no client library, wallet integration, or
     cached config can silently fall back to the old ID.
   - Confirmation once the new contract (below) is live, so clients can
     cut over.
3. **Execute the address-migration procedure now**, treating it as the
   incident response rather than routine maintenance:
   [DEPLOYMENT.md — Address migration (new contract ID)](DEPLOYMENT.md#address-migration-new-contract-id).
   That procedure's own step 3 ("pause the old contract") is **not
   possible** here — skip it and rely entirely on step 1 (public
   announcement) and client-side blocklisting for containment instead.
   Tooling to make this migration itself less manual is tracked in
   [issue #617](https://github.com/scout-off/scout-off-contracts/issues/617);
   until that lands, follow the manual steps in `DEPLOYMENT.md` directly.
4. **Replay state into the new contract** from the off-chain indexer's
   event log (DEPLOYMENT.md step 4), and audit that log for any
   attacker-issued transactions so they aren't replayed into the new
   deployment.

### Prevention

Both scenarios exist only because each contract has exactly one admin
key with no fallback. This is the strongest possible argument for
prioritizing [issue #609 — multisig/timelock admin
authorization](https://github.com/scout-off/scout-off-contracts/issues/609):
a threshold scheme turns "one lost or stolen key" into "an attacker or
accident needs to compromise/lose a quorum of keys," and a timelock gives
the team a window to react to a malicious pending action (e.g. a proposed
`upgrade()`) before it takes effect — neither of which the current
single-key model provides.

### Tabletop exercise (rehearse this without an actual key loss)

Run this against a disposable local or testnet deployment — never against
a shared testnet or mainnet contract other people rely on.

- [ ] Deploy a fresh set of four contracts (`./scripts/deploy.sh testnet`)
      and initialize them (`./scripts/initialize.sh testnet`) with a
      throwaway admin identity.
- [ ] Simulate "key lost" by discarding the throwaway identity's secret
      (e.g. remove it from `stellar keys` / your signer) without pausing
      first.
- [ ] Confirm `pause_contract`, `propose_admin`, and `upgrade` all fail as
      expected without that identity available — this is the state the
      real incident would put you in.
- [ ] Walk through the address-migration procedure end to end
      (`DEPLOYMENT.md`) against this disposable deployment, skipping the
      "pause the old contract" step to mirror Scenario B.
- [ ] Confirm the new contract is initialized, wired, and reachable, and
      that a client pointed at the new contract ID works while the old ID
      returns only its frozen, final state.
- [ ] Time the exercise. If any step took noticeably longer than expected
      or required improvising commands not in this doc, update this
      runbook and `DEPLOYMENT.md` with what you learned before the next
      drill.
- [ ] Confirm every participant knows, without looking it up, who has
      authority to make the public-announcement call in a real incident —
      this should never be discovered for the first time during a real
      compromise.

---

## Rehearse Routine Admin Rotation

Use `scripts/rehearse-admin-rotation.sh` to rehearse the two-step
`propose_admin` → `accept_admin` rotation procedure on a **disposable**
local or testnet deployment before performing it against a real shared
testnet or mainnet contract.

This is the **routine, happy-path** counterpart to the tabletop exercise in
[Emergency: Admin Key Loss / Compromise](#emergency-admin-key-loss--compromise)
above. That exercise rehearses the failure scenario; this one rehearses the
normal, successful procedure so operators have practised it at least once
before they need to do it for real (e.g. onboarding a new platform operator,
rotating keys after a team member leaves, or regular security hygiene).

### When to run this

- Before performing a routine admin rotation on a shared testnet for the
  first time.
- Before performing a rotation on mainnet.
- Any time the rotation procedure in `ai.md` or `docs/DEPLOYMENT.md` is
  updated — re-run to confirm the new steps still work end-to-end.
- As part of onboarding a new operator who will be responsible for admin
  rotations.

### Prerequisites

- `stellar-cli` installed at the pinned version (see `docs/CONTRIBUTING.md`).
- A running local Soroban quickstart sandbox **or** testnet access with
  funded accounts.
- For local: start the sandbox first (see `docs/DEPLOYMENT.md` or the
  `bindings-smoke-test` CI job for the exact `docker run` command).

### Run the rehearsal

```bash
# Against the local quickstart sandbox (default):
bash scripts/rehearse-admin-rotation.sh local

# Against Stellar testnet (will request funding via friendbot):
bash scripts/rehearse-admin-rotation.sh testnet
```

The script:
1. Generates two fresh ephemeral Stellar identities (`OLD_ADMIN` and `NEW_ADMIN`).
2. Funds both via friendbot.
3. Builds and deploys a fresh, isolated set of all four contracts.
4. Initialises them with `OLD_ADMIN`.
5. For each contract, performs `propose_admin(NEW_ADMIN)` then `accept_admin()`.
6. Verifies `NEW_ADMIN` can call `pause_contract` / `unpause_contract`
   (admin-only operations) after the rotation.
7. Verifies `OLD_ADMIN` is correctly rejected from admin-only operations.
8. Cleans up the ephemeral identities.

A `PASS` result means the full rotation procedure worked end-to-end on a
real Soroban contract deployment and you are ready to perform the same
steps on your intended target.

### After a successful rehearsal

When you are ready to rotate on a real deployment:

```bash
# Load the real contract IDs
source .env.contracts

# --- On the CURRENT admin machine: ---
stellar contract invoke --id "$REGISTRATION_CONTRACT_ID" \
  --source "$CURRENT_ADMIN_SECRET" --network "$STELLAR_NETWORK" \
  -- propose_admin --new_admin "$NEW_ADMIN_ADDRESS"

stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" \
  --source "$CURRENT_ADMIN_SECRET" --network "$STELLAR_NETWORK" \
  -- propose_admin --new_admin "$NEW_ADMIN_ADDRESS"

stellar contract invoke --id "$PROGRESS_CONTRACT_ID" \
  --source "$CURRENT_ADMIN_SECRET" --network "$STELLAR_NETWORK" \
  -- propose_admin --new_admin "$NEW_ADMIN_ADDRESS"

stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" \
  --source "$CURRENT_ADMIN_SECRET" --network "$STELLAR_NETWORK" \
  -- propose_admin --new_admin "$NEW_ADMIN_ADDRESS"

# --- On the NEW admin machine (the incoming operator): ---
stellar contract invoke --id "$REGISTRATION_CONTRACT_ID" \
  --source "$NEW_ADMIN_SECRET" --network "$STELLAR_NETWORK" \
  -- accept_admin

stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" \
  --source "$NEW_ADMIN_SECRET" --network "$STELLAR_NETWORK" \
  -- accept_admin

stellar contract invoke --id "$PROGRESS_CONTRACT_ID" \
  --source "$NEW_ADMIN_SECRET" --network "$STELLAR_NETWORK" \
  -- accept_admin

stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" \
  --source "$NEW_ADMIN_SECRET" --network "$STELLAR_NETWORK" \
  -- accept_admin
```

> **Note**: `propose_admin` stores the proposed address on-chain. The current
> admin retains all privileges until `accept_admin` is called from the new
> address — the rotation is not complete until both steps succeed on every
> contract. Confirm with `health()` and a test admin call after each
> `accept_admin` to ensure the contract is live and the new key works.

### Scope

This procedure covers the **routine, non-emergency rotation**. For the
scenario where the current admin key is lost or compromised and cannot sign
`propose_admin`, see
[Emergency: Admin Key Loss / Compromise](#emergency-admin-key-loss--compromise).

---

## Post-Incident Recovery: Unpause All Contracts

Only unpause after the root cause has been confirmed as fixed or mitigated.

### One-command unpause script

Run the emergency unpause script:

```bash
./scripts/emergency-unpause.sh
```

### Verify each contract is unpaused

```bash
source .env && source .env.contracts
NETWORK_ARGS="--network $STELLAR_NETWORK --source $ADMIN_SECRET"

echo "registration:" && stellar contract invoke --id "$REGISTRATION_CONTRACT_ID" $NETWORK_ARGS -- health
echo "verification:" && stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" $NETWORK_ARGS -- health
echo "progress:"     && stellar contract invoke --id "$PROGRESS_CONTRACT_ID"     $NETWORK_ARGS -- health
echo "scout_access:" && stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" $NETWORK_ARGS -- health
```

Expected output for each contract after a successful unpause:

```json
{"initialized":true,"paused":false}
```

---

### Scenario B — Key compromised (active attacker)

The attacker now holds sole admin rights. They can revoke every validator,
rewrite fee config, propose (and, from a second address they also control,
accept) a new admin, or push a malicious `upgrade()` — and the legitimate
team **cannot call `pause_contract` to stop any of it**, because that also
requires the now-compromised admin key.

Since the contract itself cannot be locked down, mitigation has to happen
around it:

1. **Public announcement, immediately.** Tell users the admin key for
   contract ID `$COMPROMISED_ID` is compromised and the contract should be
   considered abandoned as of now. Include the exact contract ID — this is
   the only thing that unambiguously identifies which deployment is unsafe.
2. **Client-side blocklisting.** The frontend and backend (both outside
   this repo) must stop reading from and writing to the compromised
   contract ID immediately — this is the only real "circuit breaker" left
   once `pause_contract` is unavailable. Coordination points to hand to
   those teams:
   - The exact compromised contract ID(s) (`REGISTRATION_CONTRACT_ID` /
     `VERIFICATION_CONTRACT_ID` / `PROGRESS_CONTRACT_ID` /
     `SCOUT_ACCESS_CONTRACT_ID` — whichever is affected; a compromised
     `progress` admin key is the scenario this issue was filed against,
     but the same procedure applies to any of the four).
   - A hard denylist entry so no client library, wallet integration, or
     cached config can silently fall back to the old ID.
   - Confirmation once the new contract (below) is live, so clients can
     cut over.
3. **Execute the address-migration procedure now**, treating it as the
   incident response rather than routine maintenance:
   [DEPLOYMENT.md — Address migration (new contract ID)](DEPLOYMENT.md#address-migration-new-contract-id).
   That procedure's own step 3 ("pause the old contract") is **not
   possible** here — skip it and rely entirely on step 1 (public
   announcement) and client-side blocklisting for containment instead.
   Tooling to make this migration itself less manual is tracked in
   [issue #617](https://github.com/scout-off/scout-off-contracts/issues/617);
   until that lands, follow the manual steps in `DEPLOYMENT.md` directly.
4. **Replay state into the new contract** from the off-chain indexer's
   event log (DEPLOYMENT.md step 4), and audit that log for any
   attacker-issued transactions so they aren't replayed into the new
   deployment.

### Prevention

Both scenarios exist only because each contract has exactly one admin
key with no fallback. This is the strongest possible argument for
prioritizing [issue #609 — multisig/timelock admin
authorization](https://github.com/scout-off/scout-off-contracts/issues/609):
a threshold scheme turns "one lost or stolen key" into "an attacker or
accident needs to compromise/lose a quorum of keys," and a timelock gives
the team a window to react to a malicious pending action (e.g. a proposed
`upgrade()`) before it takes effect — neither of which the current
single-key model provides.

### Tabletop exercise (rehearse this without an actual key loss)

Run this against a disposable local or testnet deployment — never against
a shared testnet or mainnet contract other people rely on.

- [ ] Deploy a fresh set of four contracts (`./scripts/deploy.sh testnet`)
      and initialize them (`./scripts/initialize.sh testnet`) with a
      throwaway admin identity.
- [ ] Simulate "key lost" by discarding the throwaway identity's secret
      (e.g. remove it from `stellar keys` / your signer) without pausing
      first.
- [ ] Confirm `pause_contract`, `propose_admin`, and `upgrade` all fail as
      expected without that identity available — this is the state the
      real incident would put you in.
- [ ] Walk through the address-migration procedure end to end
      (`DEPLOYMENT.md`) against this disposable deployment, skipping the
      "pause the old contract" step to mirror Scenario B.
- [ ] Confirm the new contract is initialized, wired, and reachable, and
      that a client pointed at the new contract ID works while the old ID
      returns only its frozen, final state.
- [ ] Time the exercise. If any step took noticeably longer than expected
      or required improvising commands not in this doc, update this
      runbook and `DEPLOYMENT.md` with what you learned before the next
      drill.
- [ ] Confirm every participant knows, without looking it up, who has
      authority to make the public-announcement call in a real incident —
      this should never be discovered for the first time during a real
      compromise.

---

## Related Documentation

- [DEPLOYMENT.md](DEPLOYMENT.md) — contract deployment order and initialization
- [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md) — full `pause_contract` / `unpause_contract` / `health` function reference
- [GLOSSARY.md](GLOSSARY.md) — domain term definitions
- [Issue #609](https://github.com/scout-off/scout-off-contracts/issues/609) — multisig/timelock admin authorization (the preventive fix for this section)
- [Issue #617](https://github.com/scout-off/scout-off-contracts/issues/617) — tooling for the address-migration procedure this section relies on

# ScoutChain Runbook

> **See also:** [Glossary](GLOSSARY.md)

Operational procedures for the ScoutChain platform.

---

## JWT Key Rotation (EdDSA Asymmetric Signing)

ScoutChain access tokens are signed with **Ed25519 (EdDSA)**. The active
private key is identified by a `kid` header; verifiers fetch public keys from
`GET /.well-known/jwks.json`.

### How rotation works (overlap-safe)

Tokens have a maximum lifetime of **15 minutes** (`MAX_ACCESS_TOKEN_TTL_SECONDS`).
To rotate without invalidating existing sessions, the old public key must remain
in `JWT_SIGNING_KEYS` until all tokens issued under it have expired.

#### Step-by-step

1. **Generate a new key pair**

   ```bash
   node -e "
   const { generateKeyPairSync } = require('crypto');
   const { privateKey, publicKey } = generateKeyPairSync('ed25519');
   console.log('PRIVATE:', privateKey.export({ type:'pkcs8', format:'pem' }).toString().replace(/\n/g,'\\n'));
   console.log('PUBLIC:',  publicKey.export({ type:'spki',  format:'pem' }).toString().replace(/\n/g,'\\n'));
   "
   ```

2. **Add the new key to `JWT_SIGNING_KEYS`** (keep the old key)

   ```
   JWT_SIGNING_KEYS=new-kid-1,<new-private-PEM>;old-kid,<old-public-PEM>
   JWT_SIGNING_ACTIVE_KID=new-kid-1
   ```

   The server now **signs** new tokens with `new-kid-1` and can still
   **verify** tokens signed with `old-kid`.

3. **Deploy** the updated environment and restart the backend.

4. **Wait** at least 15 minutes (or the value of `ACCESS_TOKEN_TTL_SECONDS`)
   for all old-signed tokens to expire naturally.

5. **Remove the old public key** from `JWT_SIGNING_KEYS`:

   ```
   JWT_SIGNING_KEYS=new-kid-1,<new-private-PEM>
   JWT_SIGNING_ACTIVE_KID=new-kid-1
   ```

6. **Deploy** again. Rotation is complete.

### JWKS endpoint cache

The endpoint sends `Cache-Control: public, max-age=900` (15 min). External
verifiers should respect this; during rotation keep the old key in the JWKS
response for the same 15-minute overlap window so cached responses remain valid.

### Emergency revocation (compromise)

If a signing key is compromised:

1. Remove the compromised private key from `JWT_SIGNING_KEYS` immediately.
2. Deploy; the server will refuse to sign new tokens with the old key.
3. All existing tokens signed by that key will be rejected (unknown `kid`).
4. Users must re-authenticate via SEP-10.

### Migration from HS256

To migrate an existing HS256 deployment:

1. Set `JWT_ALLOW_HS256_MIGRATION=true` and add `JWT_SIGNING_KEYS` with the new asymmetric key.
2. The server will sign **new** tokens asymmetrically and verify both HS256 (old) and EdDSA (new).
3. After `MAX_ACCESS_TOKEN_TTL_SECONDS` (15 min), all old HS256 tokens expire.
4. Set `JWT_ALLOW_HS256_MIGRATION=false` (or remove it) and redeploy.

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

#### Best-effort behavior

The script implements best-effort across all contracts:

- **scout_access is paused first** — since it holds funds, its safety takes priority
- **Retries transient failures** — up to 5 attempts with exponential backoff (500ms → 1s → 2s → 4s → 8s max with jitter)
- **Continues regardless of failures** — a failure on one contract does not prevent attempts on the others
- **Verifies final state** — calls `health()` on each contract after pausing to confirm `paused: true`
- **Reports clear summary** — prints a table showing pause result and health check per contract; exits non-zero if any contract is not in the desired state

#### Optional filtering

To pause only specific contracts, use the `--contracts` flag:

```bash
# Pause only registration and progress
./scripts/emergency-pause.sh --contracts registration,progress

# Pause only scout_access (funds at risk)
./scripts/emergency-pause.sh --contracts scout_access
```

Valid contract names: `registration`, `verification`, `progress`, `scout_access`

#### Example output

```bash
  Emergency Pause - Starting

Target network: testnet
Contracts to pause: registration verification progress scout_access

--- Priority: scout_access (funds at risk) ---
  [attempt 1/5] Pausing scout_access...
  ✓ scout_access paused successfully

--- Priority: registration, verification, progress ---
  [attempt 1/5] Pausing registration...
  ✓ registration paused successfully
  [attempt 1/5] Pausing verification...
  ✓ verification paused successfully
  [attempt 1/5] Pausing progress...
  ✓ progress paused successfully

--- Verifying final state via health() ---
{"initialized":true,"paused":true}
{"initialized":true,"paused":true}
{"initialized":true,"paused":true}
{"initialized":true,"paused":true}

  Emergency Pause - Summary

Contract              Pause Result    Health Check
--------              ------------    ------------
scout_access          success         paused:true
registration          success         paused:true
verification          success         paused:true
progress              success         paused:true

✓ All targeted contracts are in PAUSED state
```

If any contract fails to pause or health check fails, the summary shows the failure and the script exits with code 1.

### Player erasure while paused

`purge_player_data` on **all three** contracts (`scout_access`,
`progress`, `verification`) is an intentional exception to the pause: it
remains callable while a contract is paused, because erasure is a compliance
operation (it deletes state and moves no funds) and operators may be required
to honour erasure requests precisely during an incident. The guard set is
identical on all three contracts — `require_admin` → `require_initialized` —
so erasure is still admin-only and still fails on an uninitialised contract.
See [PLAYER_ERASURE.md](PLAYER_ERASURE.md) for the full procedure.

### Player erasure while paused

`purge_player_data` on **all three** contracts (`scout_access`,
`progress`, `verification`) is an intentional exception to the pause: it
remains callable while a contract is paused, because erasure is a compliance
operation (it deletes state and moves no funds) and operators may be required
to honour erasure requests precisely during an incident. The guard set is
identical on all three contracts — `require_admin` → `require_initialized` —
so erasure is still admin-only and still fails on an uninitialised contract.
See [PLAYER_ERASURE.md](PLAYER_ERASURE.md) for the full procedure.

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
./scripts/emergency-pause.sh --function approve_milestone

# 2. Continue validator operations (registration, revocation)
# 3. The script prints health() for the verification contract to confirm state
# 4. Once investigation complete, unpause
./scripts/emergency-unpause.sh --function approve_milestone
```

The script prints the resulting `health()` field(s) after the call, so no
separate `health` invocation is needed. If you need to run the calls by hand
(e.g. the script is unavailable), the equivalent manual commands are:

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- pause_approve_milestone

stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- health

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
./scripts/emergency-pause.sh --function pay_to_contact

# 2. Continue scout operations (subscribe, renew, read state)
# 3. The script prints health() for the scout_access contract to confirm state
# 4. Once investigation complete, unpause
./scripts/emergency-unpause.sh --function pay_to_contact
```

The script prints the resulting `health()` field(s) after the call, so no
separate `health` invocation is needed. If you need to run the calls by hand
(e.g. the script is unavailable), the equivalent manual commands are:

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- pause_pay_to_contact

stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- health

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

Run the emergency unpause script:

```bash
./scripts/emergency-unpause.sh
```

#### Best-effort behavior

The script implements best-effort across all contracts:

- **scout_access is unpaused first** — since it holds funds, its safety takes priority
- **Retries transient failures** — up to 5 attempts with exponential backoff (500ms → 1s → 2s → 4s → 8s max with jitter)
- **Continues regardless of failures** — a failure on one contract does not prevent attempts on the others
- **Verifies final state** — calls `health()` on each contract after unpause to confirm `paused: false`
- **Reports clear summary** — prints a table showing unpause result and health check per contract; exits non-zero if any contract is not in the desired state

#### Optional filtering

To unpause only specific contracts, use the `--contracts` flag:

```bash
# Unpause only registration and progress
./scripts/emergency-unpause.sh --contracts registration,progress

# Unpause only scout_access (funds at risk)
./scripts/emergency-unpause.sh --contracts scout_access
```

Valid contract names: `registration`, `verification`, `progress`, `scout_access`

#### Example output

```bash
========================================
  Emergency Unpause - Starting
========================================

Target network: testnet
Contracts to unpause: registration verification progress scout_access

--- Priority: scout_access (funds at risk) ---
  [attempt 1/5] Unpausing scout_access...
  ✓ scout_access unpaused successfully

--- Priority: registration, verification, progress ---
  [attempt 1/5] Unpausing registration...
  ✓ registration unpaused successfully
  [attempt 1/5] Unpausing verification...
  ✓ verification unpaused successfully
  [attempt 1/5] Unpausing progress...
  ✓ progress unpaused successfully

--- Verifying final state via health() ---
{"initialized":true,"paused":false}
{"initialized":true,"paused":false}
{"initialized":true,"paused":false}
{"initialized":true,"paused":false}

========================================
  Emergency Unpause - Summary
========================================

Contract              Unpause Result  Health Check
--------              --------------  ------------
scout_access          success         paused:false
registration          success         paused:false
verification          success         paused:false
progress              success         paused:false

✓ All targeted contracts are in UNPAUSED state
```

If any contract fails to unpause or health check fails, the summary shows the failure and the script exits with code 1.

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

## Emergency: Bulk Validator Revocation

Use when several validators (for example a colluding ring) must be deactivated at once.

1. Split the wallet list into batches of at most **10** (`MAX_BATCH_REVOKE`). Larger batches fail with `BatchTooLarge` before any state change.
2. Call `batch_revoke_validators(wallets, ForCause, reason)` for each batch. Every wallet is deactivated, removed from open dispute votes and excluded from attestations immediately. A `batch_revoked` summary event is emitted.
3. The cascade sweep is **not** run inline. For each revoked wallet call `continue_revocation_cascade(wallet)` repeatedly (admin only) until it emits `revocation_cascade_complete` (each call flags at most 50 milestones).
4. Verify with `is_milestone_flagged` / `get_active_validator_count`.

## Contract Upgrades

Every `upgrade(new_wasm_hash)` call in all four contracts emits a
`contract_upgraded` event **before** `update_current_contract_wasm` executes,
so the event is attributed to the old code version and is always present in the
committed ledger state.

### Event schema

| Field   | Value |
|---------|-------|
| Topics  | `("contract_upgraded", admin_address)` |
| Data    | `new_wasm_hash` (BytesN\<32\>) |

### Verifying an upgrade on-chain

After running `scripts/upgrade.sh`, confirm the event was emitted:

```bash
source .env && source .env.contracts
# Fetch the latest transaction for the contract and check for the event
stellar contract events \
  --id "$REGISTRATION_CONTRACT_ID" \
  --network "$STELLAR_NETWORK" \
  --start-ledger <ledger_before_upgrade>
```

Look for a record with topic `["contract_upgraded", "<admin_address>"]` and
data equal to the new WASM hash. Repeat for all four contract IDs. If no such
event appears, the upgrade transaction may not have been confirmed — do not
proceed until it is verified.

### What to check after every upgrade

1. Confirm the `contract_upgraded` event appears in the ledger for each contract.
2. Re-wire instance-storage links per the [DEPLOYMENT.md post-upgrade checklist](DEPLOYMENT.md).
3. Run `scripts/verify-cross-contract-wiring.sh` and `scripts/health-check.sh`.
4. Run the upgrade rehearsal tests locally: `cargo test --workspace --test upgrade_rehearsal`.

---

## Verify Deployed WASM Against a Release (issue #1614)

After any deploy or upgrade, or when an auditor asks what code is running, run:

```bash
./scripts/verify-deployed-wasm.sh <network> <release-tag>
```

It fetches the installed code for each contract ID in `.env.contracts`, hashes
it and compares it with the release's `wasm-hashes.json`, printing `MATCH` or
`MISMATCH` per contract and exiting non-zero on any mismatch. A `MISMATCH` means
the on-chain code is not the tagged build: treat it as an incident (consider the
pause procedure above) until explained. Set `WASM_HASHES_FILE` to verify against
a local manifest. See [DEPLOYMENT.md](DEPLOYMENT.md#reproducible-builds-and-deployed-wasm-verification-issue-1614).

## Solvency Monitor

The `scripts/solvency-monitor.js` script reads on-chain state from the
`scout_access` contract and verifies the invariant:

```
token.balance(contract) >= AccumulatedFees + Σ outstanding TrialEscrow.amount
```

A **deficit** means the contract cannot pay all its obligations (fees owed to
admin plus escrowed trial-offer funds owed to scouts). A **surplus** beyond the
configured tolerance may indicate a bookkeeping regression.

### Automated checks

The GitHub Actions workflow `.github/workflows/solvency-monitor.yml` runs every
6 hours against testnet. It uploads a JSON report artifact named
`solvency-report-<run_id>` with 30 days' retention. The workflow requires two
repository secrets:

| Secret | Description |
|---|---|
| `TESTNET_SCOUT_ACCESS_CONTRACT_ID` | Contract ID of `scout_access` on testnet |
| `SOLVENCY_ALERT_WEBHOOK_URL` | Slack/Discord/generic webhook URL (optional) |

### Manual run

```bash
# Against testnet (requires stellar CLI configured for testnet)
SCOUT_ACCESS_CONTRACT_ID=CXXXXXXXX \
  node scripts/solvency-monitor.js --network testnet --json

# With a surplus tolerance of 1 XLM (10_000_000 stroops)
SCOUT_ACCESS_CONTRACT_ID=CXXXXXXXX \
  node scripts/solvency-monitor.js --network testnet \
  --tolerance 10000000
```

### Responding to a deficit

A deficit means `balance < AccumulatedFees + Σ escrow`. Possible causes:

1. **Escrow bug**: a purge or refund call destroyed an escrow without returning
   the funds. Check for recent `purge_player_data` calls, then compare the
   `player_data_purged` events against `trial_offer_refunded` events.
2. **Accounting regression**: `AccumulatedFees` was incremented without a
   corresponding token transfer. Check recent fee-collection transactions.
3. **Direct token withdrawal**: `withdraw_fees` was called beyond the actual
   contract balance. This is a contract bug — pause immediately.

**Immediate response**:
1. Pause the contract (`./scripts/emergency-pause.sh`).
2. Run the solvency monitor manually with `--json` and save the report.
3. Cross-check with `scripts/audit-event-history.js` for the affected player IDs.
4. File an incident and notify the admin multi-sig before any further action.

### Responding to a surplus

An unexplained surplus may mean:

- Someone sent XLM directly to the contract address (donation / mistake).
- An escrow was cleaned up without decrementing the outstanding index.

A surplus alone does not require pausing. Investigate via `audit-event-history.js`
and document the finding. If it cannot be explained, treat it as a potential
accounting regression and escalate.

## Related Documentation

- [DEPLOYMENT.md](DEPLOYMENT.md) — contract deployment order and initialization
- [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md) — full `pause_contract` / `unpause_contract` / `health` function reference
- [GLOSSARY.md](GLOSSARY.md) — domain term definitions
- [Issue #609](https://github.com/scout-off/scout-off-contracts/issues/609) — multisig/timelock admin authorization (the preventive fix for this section)
- [Issue #617](https://github.com/scout-off/scout-off-contracts/issues/617) — tooling for the address-migration procedure this section relies on

## Migration window sequence and `scripts/upgrade.sh`

Migration state replay must run in order: `open_migration_window` → `admin_seed_*` → `close_migration_window`. Closing is **irreversible** (it seals the window), so it is rejected with `MigrationNotActive` unless the window is currently open.

`scripts/upgrade.sh` uploads the WASM once and signs every call with `--source <identity>` (or `UPGRADE_SOURCE`). Contracts without `schema_version`/`migrate` (registration, verification, scout_access) are upgraded and the migration phase is skipped with exit code 0:

```bash
UPGRADE_SOURCE=admin ./scripts/upgrade.sh --contract <id> --wasm <path> [--network testnet]
```

### Monitoring migration windows

An **open migration window is security-relevant**: while it is open, the
`admin_seed_*` entrypoints accept writes that bypass normal validation, so an
unexpectedly open window is an incident signal.

- **Alert on `migration_window_opened` in production (mainnet).** Page the
  on-call operator on any occurrence that was not scheduled as part of a
  planned upgrade. `migration_window_closed` should follow within the planned
  maintenance window; a window left open past its expected close time is also
  worth alerting on.
- `migration_window_opened` / `migration_window_closed` are emitted by
  `progress`, `verification` and `scout_access`; `progress` additionally emits
  `schema_migrated` from `migrate()`. See the
  [EVENT_REFERENCE.md entries](EVENT_REFERENCE.md#migration-window-events)
  for the exact topics/data per contract.
- **Registration is different.** It has no migration window and does not emit
  these events — it uses a separate migration-ticket mechanism instead. Do not
  expect `migration_window_*` events from the registration contract, and do not
  treat their absence there as a problem.

---

## Re-index from Archive

Use this procedure when the indexer must be bootstrapped from genesis (e.g.
after a prolonged outage, a fresh deployment, or a full audit of historical
state).

### Prerequisites

- A running Galexie / data-lake archive reachable via HTTP(S).  The archive
  must expose an `index.json` manifest and JSONL partition files as described
  in [INDEXER.md — Archival Event Source](INDEXER.md#archival-event-source).
- `GALEXIE_ARCHIVE_URL` set to the archive base URL.
- `RPC_URL`, `REGISTRATION_CONTRACT_ID`, `VERIFICATION_CONTRACT_ID`,
  `PROGRESS_CONTRACT_ID`, and `SCOUT_ACCESS_CONTRACT_ID` set (or present in
  `.env.contracts`).
- Optional: `GALEXIE_API_KEY` for authenticated buckets.

### Steps

```bash
# 1. Load environment
source .env
source .env.contracts

# 2. Verify gap detection is working (should exit 0 with archive configured)
GALEXIE_ARCHIVE_URL=https://your-archive.example.com/stellar-events \
  node scripts/audit-event-history.js --player-id 1 --from-ledger 1

# 3. Full from-genesis audit of all players
GALEXIE_ARCHIVE_URL=https://your-archive.example.com/stellar-events \
  node scripts/audit-event-history.js all --from-ledger 1

# 4. Run the reconciler to sync the indexer DB
node scripts/reconcile-indexer.js
```

### Without an archive (RPC-only)

If no archive is available, omit `GALEXIE_ARCHIVE_URL`.  The audit tool will
detect and refuse any range that starts before the RPC retention window:

```
Incomplete event range: Requested range starts at ledger 1 but the RPC node
only retains events from ledger 95000. Configure an archival event source
(GALEXIE_ARCHIVE_URL) to cover the full range, or start from ledger 95000.
```

Start from the reported ledger to audit only the retained window:

```bash
node scripts/audit-event-history.js all --from-ledger 95000
```

**Note:** An RPC-only audit of a partial range cannot guarantee tamper-proof
progress claims for history outside the window.

# ScoutChain Runbook

> **See also:** [Glossary](GLOSSARY.md)

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

> **Note**: Each `pause_contract` call is a separate Stellar transaction.
> If the script exits mid-way (e.g. network error), run it again — the already-
> paused contracts will return `ContractPaused` but will not change state.
> Continue from the failed contract manually if needed.

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

## Related Documentation

- [DEPLOYMENT.md](DEPLOYMENT.md) — contract deployment order and initialization
- [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md) — full `pause_contract` / `unpause_contract` / `health` function reference
- [GLOSSARY.md](GLOSSARY.md) — domain term definitions

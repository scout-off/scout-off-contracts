#!/usr/bin/env bash
# ScoutChain — seed testnet with demo data
# Run after initialize.sh to create test players, validators, and scouts.
set -euo pipefail

if [[ ! -f .env.contracts ]]; then
	echo "ERROR: .env.contracts not found. Run scripts/deploy.sh and scripts/initialize.sh first." >&2
	exit 1
fi

# shellcheck source=/dev/null
source .env.contracts

NETWORK="testnet"
DEPLOYER="${DEPLOYER_SECRET:?Set DEPLOYER_SECRET}"
: "${ADMIN_ADDRESS:?Set ADMIN_ADDRESS}"

fail() {
	echo "ERROR: $*" >&2
	exit 1
}

account_exists() {
	stellar account show "$1" --network "$NETWORK" >/dev/null 2>&1
}

wait_for_account() {
	local addr="$1"

	for i in $(seq 1 10); do
		if account_exists "$addr"; then
			return 0
		fi

		echo "    Waiting for $addr to be funded... ($i/10)"
		sleep 3
	done

	fail "account $addr was not funded after 30 seconds"
}

fund_account() {
	local addr="$1"

	if account_exists "$addr"; then
		echo "    $addr already exists; skipping Friendbot."
		return 0
	fi

	curl --fail --silent --show-error "https://friendbot.stellar.org?addr=$addr" >/dev/null ||
		fail "Friendbot funding failed for $addr"
	wait_for_account "$addr"
}

run_with_retries() {
	local label="$1"
	shift
	local output=""

	for i in 1 2 3; do
		if output="$("$@" 2>&1)"; then
			[[ -n "$output" ]] && echo "$output"
			return 0
		fi

		if [[ "$output" == *"AlreadyRegistered"* || "$output" == *"ValidatorAlreadyRegistered"* ]]; then
			echo "    $label already exists; skipping."
			return 0
		fi

		echo "    $label failed (attempt $i/3)." >&2
		[[ -n "$output" ]] && echo "$output" >&2
		if [[ "$i" -lt 3 ]]; then
			sleep 3
		fi
	done

	fail "$label failed after 3 attempts"
}

player_registered() {
	stellar contract invoke \
		--id "$REGISTRATION_CONTRACT_ID" \
		--network "$NETWORK" \
		-- get_player_by_wallet \
		--wallet "$PLAYER_ADDRESS" >/dev/null 2>&1
}

validator_registered() {
	local status
	status="$(
		stellar contract invoke \
			--id "$VERIFICATION_CONTRACT_ID" \
			--network "$NETWORK" \
			-- get_validator_status \
			--wallet "$VALIDATOR_ADDRESS" 2>/dev/null || true
	)"
	[[ "$status" != *"NotRegistered"* && -n "$status" ]]
}

echo "==> Generating test keypairs..."

stellar keys generate --no-fund player-test >/dev/null 2>&1 || stellar keys show player-test --secret >/dev/null
stellar keys generate --no-fund scout-test >/dev/null 2>&1 || stellar keys show scout-test --secret >/dev/null
stellar keys generate --no-fund validator-test >/dev/null 2>&1 || stellar keys show validator-test --secret >/dev/null

PLAYER_ADDRESS=$(stellar keys address player-test)
SCOUT_ADDRESS=$(stellar keys address scout-test)
VALIDATOR_ADDRESS=$(stellar keys address validator-test)

echo "    Player:    $PLAYER_ADDRESS"
echo "    Scout:     $SCOUT_ADDRESS"
echo "    Validator: $VALIDATOR_ADDRESS"

mkdir -p testnet
{
	echo "NETWORK=$NETWORK"
	echo "PLAYER_KEY_ALIAS=player-test"
	echo "PLAYER_ADDRESS=$PLAYER_ADDRESS"
	echo "PLAYER_PUBLIC_KEY=$PLAYER_ADDRESS"
	echo "SCOUT_KEY_ALIAS=scout-test"
	echo "SCOUT_ADDRESS=$SCOUT_ADDRESS"
	echo "SCOUT_PUBLIC_KEY=$SCOUT_ADDRESS"
	echo "VALIDATOR_KEY_ALIAS=validator-test"
	echo "VALIDATOR_ADDRESS=$VALIDATOR_ADDRESS"
	echo "VALIDATOR_PUBLIC_KEY=$VALIDATOR_ADDRESS"
} >testnet/.accounts

echo "==> Funding test accounts via Friendbot..."
fund_account "$PLAYER_ADDRESS"
fund_account "$SCOUT_ADDRESS"
fund_account "$VALIDATOR_ADDRESS"

echo "==> Registering validator..."
if validator_registered; then
	echo "    Validator already registered; skipping."
else
	run_with_retries "validator registration" \
		stellar contract invoke \
		--id "$VERIFICATION_CONTRACT_ID" \
		--source "$DEPLOYER" \
		--network "$NETWORK" \
		-- register_validator \
		--wallet "$VALIDATOR_ADDRESS" \
		--credentials "UEFA B License — Test Validator"
fi

echo "==> Registering test player..."
if player_registered; then
	echo "    Player already registered; skipping."
else
	run_with_retries "player registration" \
		stellar contract invoke \
		--id "$REGISTRATION_CONTRACT_ID" \
		--source player-test \
		--network "$NETWORK" \
		-- register_player \
		--wallet "$PLAYER_ADDRESS" \
		--vitals '{"age":19,"position":"Forward","region":"West Africa","nationality":"Ghana"}' \
		--ipfs_hashes '["QmTestHighlight1","QmTestPhoto1"]'
fi

echo "==> Registering test scout..."
run_with_retries "scout registration" \
	stellar contract invoke \
	--id "$REGISTRATION_CONTRACT_ID" \
	--source scout-test \
	--network "$NETWORK" \
	-- register_scout \
	--wallet "$SCOUT_ADDRESS" \
	--region "Europe"

echo ""
echo "==> Seed complete."
echo "    Player address:    $PLAYER_ADDRESS"
echo "    Scout address:     $SCOUT_ADDRESS"
echo "    Validator address: $VALIDATOR_ADDRESS"
echo ""
echo "    Account details written to testnet/.accounts."

# ScoutChain

[![Soroban Contract CI](https://github.com/your-org/scoutchain/actions/workflows/contract-ci.yml/badge.svg)](https://github.com/your-org/scoutchain/actions/workflows/contract-ci.yml)

Decentralized football talent scouting platform on Stellar — tamper-proof player profiles, on-chain progress verification, and direct scout-to-player connections powered by Soroban smart contracts.

## Overview

ScoutChain solves the visibility problem for undiscovered football talent worldwide. Players from underserved regions create dynamic on-chain profiles backed by verifiable milestones — approved by local coaches, academy directors, and certified trainers. Scouts browse a trusted, filterable talent pool and connect directly with players, with every interaction settled via Stellar's near-zero-cost payment layer.

Stellar is the backbone: transactions cost fractions of a cent and settle in 3–5 seconds, making microtransactions viable for scouts paying to unlock premium data or contact players across borders. Soroban smart contracts handle player registration, milestone verification, scout subscriptions, and secure connection agreements with auditable, tamper-proof logic.

## Features

- **Dynamic Player Profiles**: On-chain identity linked to highlight reels stored on IPFS/Arweave, with verified stats and vitals
- **Verifiable Progress Bar**: Milestones confirmed by authorized validators are written to the blockchain — no fake stats
- **Multi-Level Verification**: Four-tier trust system from unverified profile to elite scout-endorsed tier
- **Scout Discovery**: Filter players by region, position, and verified progress tier
- **Pay-to-Contact**: Scouts pay micro-fees in $XLM or platform token to unlock premium data or initiate contact
- **Validator Network**: Local coaches, academy directors, and certified trainers act as trusted on-chain validators
- **Wallet-Based Auth**: Players and scouts log in securely via Stellar wallets (Freighter, Albedo, or Lobstr) using SEP-10
- **Fractionalized Sponsorship** *(Future)*: Fans and local investors fund players via "Player Tokens" with transfer fee revenue sharing

## Architecture

```mermaid
graph TB
    subgraph Users
        P[Player]
        S[Scout]
        V[Validator — Coach / Academy]
        ADM[Platform Admin]
    end

    subgraph Frontend["Frontend (Next.js / Flutter)"]
        PP[Player Profile Dashboard]
        SD[Scout Discovery Dashboard]
        VP[Validator Approval Panel]
        AUTH[Auth — SEP-10 / Stellar Wallet]
    end

    subgraph Payments["Payment Layer"]
        XLM[Stellar XLM]
        TOKEN[Platform Token]
    end

    subgraph Contract["Smart Contracts (Soroban / Rust)"]
        REG[registration.rs — Player & scout onboarding]
        VERIFY[verification.rs — Milestone approvals]
        PROGRESS[progress.rs — On-chain progress level updates]
        SCOUT[scout_access.rs — Subscriptions & pay-to-contact]
    end

    subgraph Storage["Decentralized Storage"]
        IPFS[IPFS / Arweave — Video highlights & photos]
    end

    subgraph Stellar["Stellar Network"]
        LEDGER[Ledger]
        ORACLE[Decentralized Oracles]
    end

    P -->|upload profile + highlight reels| PP
    PP -->|store video/photos| IPFS
    IPFS -->|content hash| REG
    REG -->|register player identity| LEDGER

    V -->|approve milestone| VP
    VP --> VERIFY
    VERIFY --> PROGRESS
    PROGRESS -->|update progress level| LEDGER
    ORACLE -->|physical stats data| VERIFY

    S -->|browse & filter talent| SD
    SD -->|pay to contact / subscribe| SCOUT
    SCOUT -->|fee settlement| XLM
    XLM --> LEDGER

    AUTH -->|wallet login SEP-10| LEDGER
    ADM -->|manage validators & fees| Contract
```

### Core Components

- **registration.rs**: Handles player and scout onboarding, stores wallet address, IPFS content hashes, and basic vitals on-chain
- **verification.rs**: Processes milestone approval requests from authorized validators and emits verification events
- **progress.rs**: Manages the four-tier progress level system and updates player progress state on-chain
- **scout_access.rs**: Handles scout subscriptions, pay-to-contact flows, and connection agreement logic
- **storage.rs**: Persistent storage for player profiles, validator registry, and scout subscription records
- **events.rs**: Event emission for off-chain indexing (new profiles, milestone approvals, scout contacts)

### Progress Level Model

Progress levels are configured per player and enforced on-chain by authorized validators:

| Level | Name | Requirement |
|-------|------|-------------|
| 0 | Unverified | Player creates profile and uploads data |
| 1 | Verified Identity | KYC passed or academy confirms active club membership |
| 2 | Performance Milestones | Match footage or physical stats verified by approved third party |
| 3 | Elite Tier | Scout feedback or trial offers logged on-chain |

## Tech Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Smart Contracts | Soroban (Rust) | Player registration, progress verification, scout subscriptions, secure connection agreements |
| Frontend | React / Next.js or Flutter | Mobile/web interface for player uploads and scout talent browsing |
| Backend & Storage | Node.js + IPFS | Heavy video files and photos stored on IPFS; content hashes saved on-chain in player profiles |
| Auth SDK | Stellar SEP-10 | Secure wallet-based login for players and scouts via Freighter, Albedo, or Lobstr |

## Smart Contract Functions

### Player Functions

- `register_player(wallet, vitals, ipfs_hashes)` — Create a new on-chain player profile at Level 0
- `update_profile(player_id, ipfs_hashes)` — Update highlight reel or photo links (player auth required)
- `get_profile(player_id)` — Retrieve full player profile and current progress level

### Validator Functions

- `approve_milestone(player_id, milestone, evidence_hash)` — Confirm a player achievement and trigger progress update (validator auth required)
- `register_validator(wallet, credentials)` — Onboard a new coach, academy, or trainer as an authorized validator (admin auth required)
- `revoke_validator(wallet)` — Remove a validator from the trusted registry (admin auth required)

### Scout Functions

- `subscribe(scout_wallet, tier)` — Purchase a scout subscription to access filtered talent pool
- `pay_to_contact(player_id, scout_wallet)` — Pay micro-fee to unlock premium data or initiate direct contact
- `log_trial_offer(player_id, scout_wallet, details_hash)` — Record a trial offer on-chain, advancing player to Level 3

### Subscription Tier Access

Each tier controls which player progress levels a scout can view and what actions they can perform. These rules are enforced on-chain by the `scout_access` contract.

| Tier | Accessible Player Levels | Pay-to-Contact | Trial Offer (`log_trial_offer`) |
|------|--------------------------|----------------|---------------------------------|
| **Basic** | Level 1 (VerifiedIdentity) and above | ❌ Not available | ❌ Not available |
| **Pro** | Level 0–3 (all levels) | ✅ Available (contact fee applies) | ❌ Not available |
| **Elite** | Level 0–3 (all levels) | ✅ Available (contact fee applies) | ✅ Available (advances player to Level 3) |

**Notes:**
- A scout without any active subscription cannot call `pay_to_contact` — the contract returns `ScoutNotSubscribed` (code 6).
- An expired subscription is treated the same as no subscription — renew via `subscribe` before contacting players.
- `log_trial_offer` is restricted to **Elite** tier only; calling it with Basic or Pro returns `Unauthorized` (code 4).
- Basic tier scouts can browse and filter players at Level 1 and above but cannot contact or make trial offers.
- Subscription downgrade to a lower tier is blocked while the current subscription is active (`SubscriptionDowngradeNotAllowed`, code 12).

### Admin Functions

- `initialize(admin, platform_token, fee_config)` — One-time contract setup
- `update_fee_config(fee_config)` — Adjust subscription and contact fee rates (admin only)
- `withdraw_fees(to)` — Withdraw accumulated platform fees (admin only)
- `pause_contract()` / `unpause_contract()` — Emergency circuit breaker (admin only)

### Query Functions

- `get_player(player_id)` — Full player profile with progress level and IPFS links
- `get_progress_history(player_id)` — Tamper-proof timeline of milestone approvals
- `filter_players(region, position, min_level)` — Scout discovery query
- `get_validators()` — Active validator registry
- `health()` — On-chain health check

## Progress Verification Flow

```
[ Player Uploads Video ]
         │
         ▼
[ Local Coach / Validator Approves ]
         │
         ▼
[ Soroban Smart Contract Updates Progress Level ] ──► [ Reflects on Scout Dashboard ]
```

### Milestone Examples

- "Scored 5 goals in Local Cup" → Level 2 milestone, approved by registered coach
- "Top speed clocked at 32 km/h" → Level 2 milestone, approved by certified trainer
- "Trial offer received from FC Example" → Level 3 milestone, logged by scout

## Player Lifecycle — Sequence Diagram

```mermaid
sequenceDiagram
    actor Player
    actor Validator
    actor Scout
    participant Contract as ScoutChain Contract
    participant Storage as IPFS / Arweave

    rect rgb(235, 245, 255)
        Note over Player,Storage: Profile creation
        Player->>Storage: upload highlight reel + photos
        Storage-->>Player: content_hash (CID)
        Player->>Contract: register_player(vitals, content_hash)
        Contract-->>Player: player_id, Level 0
    end

    rect rgb(240, 255, 240)
        Note over Validator,Contract: Milestone verification
        Validator->>Contract: approve_milestone(player_id, milestone, evidence)
        Contract->>Contract: validate validator authorization
        Contract-->>Player: progress level updated (Level 1 → 2)
    end

    rect rgb(245, 235, 255)
        Note over Scout,Contract: Scout discovery & contact
        Scout->>Contract: filter_players(region, position, min_level)
        Contract-->>Scout: matching player list
        Scout->>Contract: pay_to_contact(player_id)
        Contract->>Contract: settle XLM fee, unlock contact details
        Contract-->>Scout: player contact info + full profile
    end

    rect rgb(255, 245, 235)
        Note over Scout,Contract: Trial offer
        Scout->>Contract: log_trial_offer(player_id, details_hash)
        Contract-->>Player: progress updated to Level 3 (Elite Tier)
    end
```

## Player Progress — State Machine

```
┌──────────────┐
│  Level 0     │  ← Profile created, data uploaded (Unverified)
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Level 1     │  ← Identity verified by academy or KYC
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Level 2     │  ← Performance milestones verified by approved third party
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Level 3     │  ← Scout feedback or trial offer logged (Elite Tier)
└──────────────┘
```

### Valid Transitions

| From | To | Trigger |
|------|----|---------|
| Level 0 | Level 1 | Validator calls `approve_milestone` — identity confirmed |
| Level 1 | Level 2 | Validator calls `approve_milestone` — performance stats verified |
| Level 2 | Level 3 | Scout calls `log_trial_offer` — trial or feedback recorded |

## Security Features

1. **Tamper-Proof History**: Every milestone approval is an immutable on-chain transaction — scouts see exactly when and how a player progressed
2. **Authorized Validators Only**: Only admin-registered validators can approve milestones, preventing self-reported fake stats
3. **Atomic Fee Settlement**: Scout contact fees and token transfers settle in a single transaction
4. **Authorization Checks**: All state-changing operations require proper Stellar account authorization
5. **Overflow Protection**: Safe arithmetic throughout all fee calculations
6. **Circuit Breaker**: Admin can pause the contract in an emergency without losing state

## Repository Structure

```
scoutchain-contracts/
├── contracts/
│   ├── registration/       # Player & scout on-chain identity
│   ├── verification/       # Validator registry & milestone approvals
│   ├── progress/           # Four-tier level state machine
│   └── scout_access/       # Subscriptions, pay-to-contact, trial offers
├── bindings/               # Auto-generated TypeScript clients (post-deploy)
│   ├── registration/
│   ├── verification/
│   ├── progress/
│   └── scout_access/
├── migrations/
│   └── 001_initial_schema.sql   # PostgreSQL schema for the backend indexer
├── scripts/
│   ├── setup-testnet.sh    # One-command full testnet setup
│   ├── deploy.sh           # Build, optimize, and deploy all contracts
│   ├── initialize.sh       # Initialize contracts + wire cross-contract link
│   └── generate-bindings.sh # Generate TypeScript clients from deployed WASMs
├── testnet/
│   └── seed.sh             # Fund test accounts and register demo data
├── config/
│   ├── testnet.json        # Testnet RPC, Horizon, and token addresses
│   └── mainnet.json        # Mainnet config (fill in RPC key before use)
├── docs/
│   ├── DEPLOYMENT.md       # Step-by-step deployment guide
│   ├── CONTRACT_REFERENCE.md # Full function reference for all contracts
│   └── CONTRIBUTING.md     # PR checklist and contribution guidelines
├── .env.example            # Environment variable template
├── ai.md                   # Cross-repo integration guide for AI assistants
└── Cargo.toml              # Workspace manifest
```

## Quick Start

### One command (recommended)

```bash
cp .env.example .env
# Fill in all six environment variables from .env.example
./scripts/setup-testnet.sh
```

This runs all five steps automatically: build → deploy → initialize → generate bindings → seed demo data. Contract IDs are saved to `.env.contracts`, TypeScript bindings to `bindings/`, and test account addresses to `testnet/.accounts`.

### Manual setup

#### 1. Prerequisites

```bash
# Rust with WASM target
rustup target add wasm32-unknown-unknown

# Stellar CLI
# https://developers.stellar.org/docs/tools/developer-tools/cli/install-stellar-cli
```

#### 2. Configure environment

```bash
cp .env.example .env
# Fill in all six required environment variables
```

#### 3. Build and deploy

```bash
./scripts/deploy.sh testnet
# Contract IDs written to .env.contracts
```

#### 4. Initialize and wire contracts

```bash
./scripts/initialize.sh testnet
# Initializes all four contracts and wires the verification → progress
# cross-contract link so approve_milestone advances levels atomically
```

#### 5. Generate TypeScript bindings

```bash
./scripts/generate-bindings.sh testnet
# Bindings written to bindings/{contract}/
# Import these in the backend and frontend repos
```

#### 6. Seed demo data (optional)

```bash
./testnet/seed.sh
# Creates funded test player, scout, and validator on testnet
```

## Cross-Contract Wiring

`approve_milestone` in the verification contract cross-calls `advance_level` in the progress contract atomically — both state changes happen in the same Stellar transaction. This is wired up by `initialize.sh` automatically:

```bash
stellar contract invoke \
  --id $VERIFICATION_CONTRACT_ID \
  -- set_progress_contract \
  --progress_contract $PROGRESS_CONTRACT_ID
```

Without this step, milestones are recorded but player levels do not advance.

## TypeScript Bindings

After deployment, run `./scripts/generate-bindings.sh testnet` to produce auto-generated TypeScript clients in `bindings/`. The backend and frontend import these directly:

```typescript
import { Client as RegistrationClient } from "@scoutchain/bindings-registration";
import { Client as ProgressClient }     from "@scoutchain/bindings-progress";
```

See `bindings/README.md` for usage details.

## Database Schema

`migrations/001_initial_schema.sql` creates the nine PostgreSQL tables the backend event indexer needs:

| Table | Purpose |
|-------|---------|
| `players` | Cached player profiles, indexed by region/position/level for fast filtering |
| `scouts` | Scout profiles |
| `validators` | Trusted validator registry |
| `milestones` | Approved milestone records per player |
| `scout_subscriptions` | Active subscription records |
| `contact_records` | Pay-to-contact audit log |
| `trial_offers` | On-chain trial offer records |
| `fee_withdrawals` | Platform fee withdrawal audit log |
| `indexer_cursor` | Horizon event stream checkpoint (single row) |

Run it against your backend PostgreSQL instance:

```bash
psql $DATABASE_URL -f migrations/001_initial_schema.sql
```



1. **Player Onboarding**
   - Connect Freighter wallet via SEP-10
   - Fill out profile: age, position, location, highlight reel links
   - Upload videos/photos to IPFS; content hashes saved on-chain
   - Profile starts at Level 0 (Unverified)

2. **Milestone Verification**
   - Local coach or academy director reviews footage or physical stats
   - Validator calls `approve_milestone` — transaction written to blockchain
   - Player's progress level updates automatically on the scout dashboard

3. **Scout Discovery**
   - Scout subscribes or pays per contact using $XLM or platform token
   - Filters talent by region, position, and minimum verified level
   - Views tamper-proof progress history before committing to a trial

4. **Trial & Elite Tier**
   - Scout logs a trial offer on-chain via `log_trial_offer`
   - Player advances to Level 3 (Elite Tier)
   - Connection agreement recorded as an immutable on-chain event

5. **Admin / Validator Management**
   - Admin registers trusted validators (coaches, academies, trainers)
   - Admin adjusts fee config and withdraws accumulated platform revenue
   - Emergency `pause_contract` available as a circuit breaker

## Configuration

Copy `.env.example` to `.env` and fill in all required values before running any script:

| Variable | Description |
|----------|-------------|
| `DEPLOYER_SECRET` | Stellar secret key used to deploy and invoke contracts |
| `ADMIN_ADDRESS` | Stellar G-address that will own all four contracts |
| `XLM_TOKEN_ADDRESS` | Native XLM token contract address on the target network |
| `STELLAR_NETWORK` | Target network: `testnet` or `mainnet` (default: `testnet`) |
| `HORIZON_URL` | Stellar Horizon endpoint for the target network |
| `SOROBAN_RPC_URL` | Soroban RPC endpoint for the target network |

Network-specific addresses are in `config/testnet.json` and `config/mainnet.json`.

After deployment, contract IDs are written to `.env.contracts` and must be copied into the backend and frontend repos:

```env
REGISTRATION_CONTRACT_ID=
VERIFICATION_CONTRACT_ID=
PROGRESS_CONTRACT_ID=
SCOUT_ACCESS_CONTRACT_ID=
```

### Mainnet Deployment Safety

When deploying to mainnet, **always verify** `config/mainnet.json` has been updated with real values before running `./scripts/deploy.sh mainnet`. The deployment script will reject the operation if placeholder values remain. Additionally:

1. Test the full deployment flow on testnet first
2. Verify all addresses in `.env` are correct for mainnet
3. Confirm `ADMIN_ADDRESS` is the intended account — ownership cannot be transferred after initialization
4. Double-check the `XLM_TOKEN_ADDRESS` matches the mainnet address (not testnet)

## Testing

```bash
# Run all contract tests
cargo test --workspace

# Run with output (useful for debugging)
cargo test --workspace -- --nocapture

# Lint and format check
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

Contract test coverage:

- ✅ Player registration, duplicate prevention, profile updates
- ✅ Scout registration
- ✅ Validator registration, revocation, and active state checks
- ✅ Milestone approval — happy path, multiple milestones per player
- ✅ Revoked validator cannot approve milestones
- ✅ Unregistered validator cannot approve milestones
- ✅ Progress level sequence (Unverified → VerifiedIdentity → PerformanceMilestones → EliteTier)
- ✅ Cannot exceed EliteTier
- ✅ Progress history entries recorded per level change
- ✅ Scout subscription — Basic, Pro, Elite tiers with XLM fee settlement
- ✅ Pay-to-contact with active subscription
- ✅ Duplicate contact prevention
- ✅ Contact without subscription fails
- ✅ Subscription expiry enforcement
- ✅ Trial offer logging (Elite only)
- ✅ Trial offer rejected for non-Elite tier
- ✅ Fee accumulation and admin withdrawal
- ✅ Pause / unpause circuit breaker

## MVP Scope

The initial testnet MVP focuses on a single end-to-end flow:

1. One player registers a profile → contract stores identity and IPFS links at Level 0
2. One validator approves a milestone → progress updates to Level 1 or 2 on-chain
3. One scout pays to contact the player → fee settles in XLM, contact details unlocked

Secondary features (fractionalized sponsorship, oracle integrations, advanced filtering) ship in subsequent milestones.

## Roadmap

- [x] Workspace scaffold — four Soroban contracts with full type, error, and event modules
- [x] Player & scout registration contract with duplicate prevention and IPFS hash storage
- [x] Validator registry with credential tracking and active/revoked state
- [x] Milestone approval with on-chain evidence hashes
- [x] Four-tier progress level state machine with immutable history
- [x] Cross-contract wiring — `approve_milestone` atomically calls `progress.advance_level`
- [x] Scout subscriptions (Basic / Pro / Elite) with XLM fee settlement
- [x] Pay-to-contact with duplicate prevention and fee accumulation
- [x] Trial offer logging (Elite tier only)
- [x] Admin fee withdrawal and circuit breaker on all contracts
- [x] Full unit test coverage across all four contracts
- [x] CI pipeline — build, test, clippy, and format check on every PR
- [x] Deployment scripts — deploy, initialize, wire, and one-command setup
- [x] TypeScript binding generation script
- [x] PostgreSQL migration schema for the backend event indexer
- [x] Testnet seed script with Friendbot-funded demo accounts
- [x] Network config files (testnet + mainnet)
- [x] Cross-repo `ai.md` integration guide
- [ ] Scout subscription and pay-to-contact flow (backend + frontend)
- [ ] Trial offer logging UI and Level 3 advancement (backend + frontend)
- [ ] Decentralized oracle integration for physical stats
- [ ] Fractionalized Player Token sponsorship model
- [ ] Mobile-first Flutter frontend
- [ ] Security audit
- [ ] Mainnet launch

## Dependencies

- `soroban-sdk = "25.3.1"` — Soroban smart contract SDK (all four contracts)
- `stellar-cli` — Stellar CLI for deployment and contract invocation
- `wasm32-unknown-unknown` — Rust compilation target for Soroban WASM output

Frontend and backend dependencies live in their respective repos (`scoutchain-frontend`, `scoutchain-backend`).

## Error Codes

Each contract defines its own error enum. The same numeric code can mean different things in different contracts — always check which contract you are calling. See [`docs/CONTRACT_REFERENCE.md`](docs/CONTRACT_REFERENCE.md) for the full per-contract reference.

### `ScoutChainError` (registration contract)

| Code | Variant | Common Cause | Resolution |
|------|---------|--------------|------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once | No action; contract is already ready |
| 2 | `NotInitialized` | Operation before `initialize` | Admin must call `initialize` first |
| 3 | `PlayerNotFound` | Invalid `player_id` | Verify the `player_id` from the registration transaction |
| 4 | `ValidatorNotAuthorized` | Unregistered account approving milestone | Admin must register the validator first |
| 5 | `InvalidProgressTransition` | Skipping or reversing a level | Follow valid 0→1→2→3 transition order |
| 6 | `ScoutNotSubscribed` | Scout has no subscription | Call `subscribe` with a valid tier and fee |
| 7 | `InsufficientFee` | Underpaying contact fee | Check current fee via `get_fee_config` |
| 8 | `AlreadyRegistered` | Wallet already has a profile for this role | Use the existing profile |
| 9 | `ContractPaused` | Circuit breaker is active | Wait for admin to call `unpause_contract` |
| 10 | `Unauthorized` | Wrong account for a privileged operation | Confirm you are using the correct Stellar account |
| 11 | `Overflow` | Counter or fee arithmetic overflowed | Use amounts within safe range |
| 12 | `ScoutNotFound` | Invalid `scout_id` | Verify the `scout_id` from the registration transaction |
| 13 | `InvalidInput` | Field too long, bad hash count, or empty value | Check field length limits in the function docs |

### `VerificationError` (verification contract)

| Code | Variant | Common Cause | Resolution |
|------|---------|--------------|------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once | No action; contract is already ready |
| 2 | `NotInitialized` | Operation before `initialize` | Admin must call `initialize` first |
| 3 | `ContractPaused` | Circuit breaker is active | Wait for admin to call `unpause_contract` |
| 4 | `Unauthorized` | Wrong account for a privileged operation | Confirm you are using the correct Stellar account |
| 5 | `ValidatorNotFound` | Wallet not in validator registry | Admin must call `register_validator` first |
| 6 | `ValidatorInactive` | Validator has been revoked | Contact admin to re-activate |
| 7 | `ValidatorAlreadyRegistered` | Wallet already registered as validator | Use the existing validator record |
| 8 | `PlayerNotFound` | Invalid `player_id` | Verify the `player_id` from the registration contract |
| 9 | `InvalidInput` | Bad evidence hash or credentials too long | Check CID format and byte limits |
| 10 | `ReasonTooLong` | Revocation reason exceeds 128 bytes | Shorten the reason string |
| 11 | `AlreadyConfigured` | `set_progress_contract` called twice | Use `update_progress_contract` for re-wiring |
| 12 | `ProgressCallFailed` | Cross-contract `advance_level` failed | Verify the progress contract is deployed and wired |
| 13 | `Overflow` | Milestone counter overflowed | Contact admin |
| 14 | `MilestoneNotFound` | Index out of range | Verify index against `get_milestone_count` |

### `ProgressError` (progress contract)

| Code | Variant | Common Cause | Resolution |
|------|---------|--------------|------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once | No action; contract is already ready |
| 2 | `NotInitialized` | Operation before `initialize` | Admin must call `initialize` first |
| 3 | `ContractPaused` | Circuit breaker is active | Wait for admin to call `unpause_contract` |
| 4 | `Unauthorized` | Wrong account for a privileged operation | Confirm you are using the correct Stellar account |
| 5 | `InvalidProgressTransition` | Level skip or reversal attempted | Follow valid 0→1→2→3 transition order |
| 6 | `AlreadyAtMaxLevel` | Player is already at `EliteTier` | No further advancement possible |
| 7 | `PlayerNotFound` | History index out of range | Verify index against `get_history_count` |
| 8 | `Overflow` | History counter overflowed | Contact admin |

### `ScoutAccessError` (scout_access contract)

| Code | Variant | Common Cause | Resolution |
|------|---------|--------------|------------|
| 1 | `AlreadyInitialized` | `initialize` called more than once | No action; contract is already ready |
| 2 | `NotInitialized` | Operation before `initialize` | Admin must call `initialize` first |
| 3 | `ContractPaused` | Circuit breaker is active | Wait for admin to call `unpause_contract` |
| 4 | `Unauthorized` | Wrong account or non-Elite tier for trial offer | Confirm account and subscription tier |
| 5 | `InsufficientFee` | Zero accumulated fees on withdrawal | Ensure fees have been collected before withdrawing |
| 6 | `ScoutNotSubscribed` | No subscription record found | Call `subscribe` with a valid tier and fee |
| 7 | `SubscriptionExpired` | Subscription past `expires_at` | Renew subscription via `subscribe` |
| 8 | `AlreadyContacted` | Duplicate `pay_to_contact` for same player | Contact is already unlocked |
| 9 | `InvalidTier` | Unknown subscription tier | Use `Basic`, `Pro`, or `Elite` |
| 10 | `Overflow` | Fee accumulation arithmetic overflowed | Contact admin |
| 11 | `TrialOfferNotFound` | Index out of range | Verify index against `get_trial_count` |
| 12 | `SubscriptionDowngradeNotAllowed` | Downgrade attempted while subscription is active | Wait for current subscription to expire |
| 14 | `ProgressCallFailed` | Cross-contract `advance_level` failed | Verify the progress contract is deployed and wired |
| 15 | `InvalidInput` | Zero or negative fee field in `FeeConfig` | All fee fields and `sub_duration_secs` must be > 0 |
| 16 | `NoFeesToWithdraw` | No accumulated fees to withdraw | Ensure fees have been collected before withdrawing |
| 17 | `UpgradeTooSoon` | `subscribe` called before minimum interval elapsed | Wait at least 1 hour between subscribe calls |

## Events

| Event | Emitted When |
|-------|-------------|
| `player_registered` | New player profile created on-chain |
| `milestone_approved` | Validator confirms a player achievement |
| `progress_updated` | Player advances to a new level |
| `scout_subscribed` | Scout purchases a talent access subscription |
| `player_contacted` | Scout pays to unlock player contact details |
| `trial_offer_logged` | Scout records a trial offer, advancing player to Level 3 |
| `fees_withdrawn` | Admin withdraws accumulated platform fees |

## Why Stellar

- Microtransactions: Scouts pay tiny fees to unlock data or contact players directly — no hefty banking fees across borders (e.g., a scout in Europe paying to contact a player in South America or Africa)
- Speed and Cost: Transactions cost fractions of a cent and settle in 3–5 seconds, ensuring a smooth experience for young players on mobile phones
- Fractionalized Sponsorship *(Future)*: Fans or local investors buy "Player Tokens" to fund a player's boots, travel, and training. If the player turns professional, a percentage of their transfer fee routes back to token holders via Stellar smart contracts

## License

MIT

## Documentation

- [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) — full deployment guide including mainnet checklist
- [docs/CONTRACT_REFERENCE.md](docs/CONTRACT_REFERENCE.md) — complete function reference for all four contracts
- [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) — PR checklist and contribution guidelines
- [ai.md](ai.md) — cross-repo integration guide for AI assistants and new team members

## Support

- GitHub Issues: [Create an issue](https://github.com/your-org/scoutchain/issues)
- Stellar Discord: https://discord.gg/stellar
- Stellar Developers: https://developers.stellar.org

## Contributing

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) for the full guide.

Quick checklist:
- All contract tests pass: `cargo test --workspace`
- Zero clippy warnings: `cargo clippy --workspace -- -D warnings`
- Formatting clean: `cargo fmt --all -- --check`
- New functions have tests and are documented in [docs/CONTRACT_REFERENCE.md](docs/CONTRACT_REFERENCE.md)
- Validator authorization logic changes require explicit review from a second team member

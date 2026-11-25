//! Property-based erasure-completeness test — issue #1774.
//!
//! Builds randomised player histories across **all four** ScoutChain
//! contracts, runs the documented erasure procedure
//! (`docs/PLAYER_ERASURE.md`) to completion for one target player, and then
//! **enumerates the real ledger storage** of every contract to prove that no
//! key referencing that player survives — except the explicitly documented
//! "Retained" allow-list.
//!
//! ## Why this is a property test
//!
//! The only existing erasure test (`player_erasure_e2e.rs`) checks a handful
//! of hand-picked keys. Several purge bugs on `main` survived precisely
//! because a hand-picked list cannot notice keys the author forgot to check
//! (scout_access orphaning later trial-offer pages, progress leaving
//! `HistoryFrontier`, verification leaving `DisputeVote` /
//! `MilestoneAttestors` / `PlayerOpenDisputeCount`, …). Instead of asserting
//! against a fixed key list, this test:
//!
//! 1. generates a random scenario (milestones, disputes, jury votes, k-of-n
//!    attestations, trial offers of mixed lifecycle state, contacts, evidence
//!    grants, progress-history length);
//! 2. executes it against a fully-wired four-contract harness;
//! 3. runs `purge_player_data` to completion on each contract, looping on
//!    `(next_cursor, more)` exactly as the doc prescribes;
//! 4. asserts token-balance conservation across the procedure; and
//! 5. walks the host's real stored ledger entries for each contract (filtered
//!    by contract address + durability), decodes every key as that
//!    contract's `DataKey`, and fails if any decoded key embeds the erased
//!    `player_id` while not being on the allow-list.
//!
//! The allow-list below mirrors the "Retained keys" table in
//! `docs/PLAYER_ERASURE.md` verbatim — that doc is the single source of
//! truth.
//!
//! Run: `cargo test -p scoutchain-chaos-tests --test erasure_completeness`
//! (also runs under `cargo test --workspace`).

use proptest::prelude::*;
use scoutchain_progress::{DataKey as ProgKey, ProgressContract, ProgressContractClient};
use scoutchain_registration::{
    DataKey as RegKey, PlayerVitals, RegistrationContract, RegistrationContractClient,
};
use scoutchain_scout_access::{
    DataKey as SaKey, FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{DataKey as VerKey, VerificationContract, VerificationContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{self, StellarAssetClient},
    xdr::{
        ContractDataDurability, ContractDataEntry, LedgerEntry, LedgerEntryData, LedgerKey,
        LedgerKeyContractData, ScAddress,
    },
    Address, Env, String, TryFromVal, Val, Vec,
};

// ── Constants ──────────────────────────────────────────────────────────────

/// Fee model used by the harness (stroops). The subscription duration is set
/// deliberately huge so the many ledger-time bumps the trial-offer cooldown
/// requires can never expire a scout's subscription mid-scenario.
const CONTACT_FEE: i128 = 100_000;
const BASIC_SUB: i128 = 1_000_000;
const PRO_SUB: i128 = 3_000_000;
const ELITE_SUB: i128 = 7_000_000;
const TRIAL_ESCROW: i128 = 500_000;
/// Trial-offer per-(scout, player) cooldown (24 h). The builder advances ledger
/// time past this between successive logs so a small scout pool can file many
/// offers.
const TRIAL_COOLDOWN_SECS: u64 = 86_400;
/// Trial-offer escrow expiry (2 h) — from the default FeeConfig used elsewhere.
const TRIAL_EXPIRY_SECS: u64 = 7_200;
/// XLM minted to every scout — ample to cover elite subscription, all contact
/// fees and every escrow they file in a single scenario.
const SCOUT_XLM_MINT: i128 = 500_000_000;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: CONTACT_FEE,
        basic_sub_stroops: BASIC_SUB,
        pro_sub_stroops: PRO_SUB,
        elite_sub_stroops: ELITE_SUB,
        // 1000 days — effectively never expires during a test scenario.
        sub_duration_secs: 1_000 * 24 * 60 * 60,
        pro_contact_limit: 50,
        trial_offer_escrow_stroops: TRIAL_ESCROW,
        trial_offer_expiry_secs: TRIAL_EXPIRY_SECS,
    }
}

// ── Allow-list (mirrors docs/PLAYER_ERASURE.md "Retained keys" table) ────────
//
// For scout_access the following player-keyed entries are intentionally
// retained after a complete purge (billing / contact / evidence-access audit
// trail). For registration, progress and verification the retained set is
// empty — a purged player leaves no player-keyed entry behind in those
// contracts.

/// `true` iff `k` is one of the documented retained entries for scout_access.
fn sa_retained(k: &SaKey) -> bool {
    matches!(
        k,
        SaKey::ContactRecord(_, _)
            | SaKey::PlayerContacts(_)
            | SaKey::EvidenceAccessGrant(_, _)
            | SaKey::EvidenceAccessGrantCount(_)
            | SaKey::EvidenceAccessGrantPage(_, _)
    )
}

/// Returns `Some(pid)` if the decoded key embeds a `player_id` field, else
/// `None`. The field that is a *player_id* (as opposed to a scout_id, nonce,
/// month bucket, day bucket, …) is identified per variant — this is the
/// contract-specific knowledge the "decode DataKey and check whether it embeds
/// the erased player_id" oracle needs.
fn reg_pid(k: &RegKey) -> Option<u64> {
    match k {
        RegKey::Player(p) | RegKey::PlayerLevel(p) | RegKey::PlayerDeactivated(p) => Some(*p),
        _ => None,
    }
}

fn prog_pid(k: &ProgKey) -> Option<u64> {
    match k {
        ProgKey::PlayerLevel(p)
        | ProgKey::HistoryCounter(p)
        | ProgKey::HistoryEntry(p, _)
        | ProgKey::HistoryVec(p)
        | ProgKey::HistoryPage(p, _)
        | ProgKey::HistoryRoot(p) => Some(*p),
        _ => None,
    }
}

fn ver_pid(k: &VerKey) -> Option<u64> {
    match k {
        VerKey::MilestoneCounter(p)
        | VerKey::Milestone(p, _)
        | VerKey::PlayerAffiliations(p)
        | VerKey::MilestoneDispute(p, _)
        | VerKey::PlayerDisputes(p)
        | VerKey::PendingMilestoneClaim(p, _)
        | VerKey::PendingMilestoneVote(p, _, _, _)
        | VerKey::DisputeVote(p, _, _)
        | VerKey::DisputeVoteCount(p, _)
        | VerKey::MilestonePendingReReview(p, _) => Some(*p),
        // Keyed by (validator, player_id) — the u64 *is* the player_id.
        VerKey::ValidatorPlayerMilestoneCount(_, p) => Some(*p),
        _ => None,
    }
}

fn sa_pid(k: &SaKey) -> Option<u64> {
    match k {
        SaKey::ContactRecord(p, _)
        | SaKey::TrialCounter(p)
        | SaKey::TrialOffer(p, _)
        | SaKey::TrialOfferLastSent(_, p)
        | SaKey::PlayerContacts(p)
        | SaKey::TrialEscrow(p, _)
        | SaKey::EvidenceAccessGrant(p, _)
        | SaKey::EvidenceAccessGrantCount(p)
        | SaKey::EvidenceAccessGrantPage(p, _) => Some(*p),
        // ContactCount(Address, u64) — u64 is a month bucket, NOT a player_id.
        // ExpiryBucket(u64) — u64 is a day bucket.
        _ => None,
    }
}

// ── Real-storage enumeration oracle ────────────────────────────────────────
//
// Walks the host's actual stored ledger entries (the same source the SDK's
// `testutils::storage::*::all()` consults) and filters by contract address
// and durability. This is a true storage enumeration — the oracle never
// consults a hand-picked key list, so it catches any player-keyed entry the
// purge forgot to remove.

/// Every ContractData key (persistent or temporary) stored under `contract`,
/// returned as the raw decoded `Val`. Filters by contract address so each
/// contract's storage is attributed to that contract alone.
fn contract_storage_keys(env: &Env, contract: &Address) -> std::vec::Vec<Val> {
    let target: ScAddress = contract.try_into().unwrap();
    let entries = env.host().get_stored_entries().unwrap();
    let mut out: std::vec::Vec<Val> = std::vec::Vec::new();
    for entry in entries {
        let (k, Some((v, _))) = entry else {
            continue;
        };
        let LedgerKey::ContractData(LedgerKeyContractData {
            ref contract, ..
        }) = *k
        else {
            continue;
        };
        if contract != &target {
            continue;
        }
        let LedgerEntry {
            data:
                LedgerEntryData::ContractData(ContractDataEntry {
                    ref key, durability, ..
                }),
            ..
        } = *v
        else {
            continue;
        };
        if !matches!(
            durability,
            ContractDataDurability::Persistent | ContractDataDurability::Temporary
        ) {
            continue;
        }
        if let Ok(kv) = Val::try_from_val(env, key) {
            out.push(kv);
        }
    }
    out
}

/// Outcome of inspecting one decoded key against the allow-list for its
/// contract.
#[derive(Debug)]
enum KeyCheck {
    /// Key does not embed the erased player_id — fine regardless.
    NotPlayer,
    /// Key embeds the erased player_id and is on the documented allow-list.
    Retained,
    /// Key embeds the erased player_id and is NOT retained — a leak.
    Leak(std::string::String),
}

fn check_reg(k: &RegKey, pid: u64) -> KeyCheck {
    match reg_pid(k) {
        Some(p) if p == pid => KeyCheck::Leak(format!("registration (pid={pid})")),
        _ => KeyCheck::NotPlayer,
    }
}
fn check_prog(k: &ProgKey, pid: u64) -> KeyCheck {
    match prog_pid(k) {
        Some(p) if p == pid => KeyCheck::Leak(format!("progress (pid={pid})")),
        _ => KeyCheck::NotPlayer,
    }
}
fn check_ver(k: &VerKey, pid: u64) -> KeyCheck {
    match ver_pid(k) {
        Some(p) if p == pid => KeyCheck::Leak(format!("verification (pid={pid})")),
        _ => KeyCheck::NotPlayer,
    }
}
fn check_sa(k: &SaKey, pid: u64) -> KeyCheck {
    match sa_pid(k) {
        Some(p) if p == pid => {
            if sa_retained(k) {
                KeyCheck::Retained
            } else {
                KeyCheck::Leak(format!("scout_access (pid={pid})"))
            }
        }
        _ => KeyCheck::NotPlayer,
    }
}

/// Run the completeness oracle over all four contracts. Returns the list of
/// leaks (empty on success).
fn find_leaks(
    env: &Env,
    addresses: &ContractAddresses,
    pid: u64,
) -> std::vec::Vec<std::string::String> {
    let mut leaks: std::vec::Vec<std::string::String> = std::vec::Vec::new();

    for kv in contract_storage_keys(env, &addresses.registration) {
        if let Ok(k) = RegKey::try_from_val(env, &kv) {
            if let KeyCheck::Leak(l) = check_reg(&k, pid) {
                leaks.push(l);
            }
        }
    }
    for kv in contract_storage_keys(env, &addresses.progress) {
        if let Ok(k) = ProgKey::try_from_val(env, &kv) {
            if let KeyCheck::Leak(l) = check_prog(&k, pid) {
                leaks.push(l);
            }
        }
    }
    for kv in contract_storage_keys(env, &addresses.verification) {
        if let Ok(k) = VerKey::try_from_val(env, &kv) {
            if let KeyCheck::Leak(l) = check_ver(&k, pid) {
                leaks.push(l);
            }
        }
    }
    for kv in contract_storage_keys(env, &addresses.scout_access) {
        if let Ok(k) = SaKey::try_from_val(env, &kv) {
            match check_sa(&k, pid) {
                KeyCheck::Leak(l) => leaks.push(l),
                _ => {}
            }
        }
    }

    leaks
}

// ── Scenario generation ────────────────────────────────────────────────────

/// Lifecycle state a logged trial offer is driven into before the purge.
#[derive(Debug, Clone, Copy)]
enum TrialKind {
    /// `confirm_trial_offer` while in-window (may fail if the player is already
    /// at the top tier — that still exercises the path and leaves the escrow
    /// for the purge to refund).
    Confirm,
    /// Past expiry, then swept by `expire_trial_offers` (refunds the scout).
    Expire,
    /// Rescued by `admin_refund_trial_escrow` (refunds the scout).
    Refund,
    /// Left outstanding — the purge itself refunds the escrow.
    Open,
}

#[derive(Debug, Clone)]
struct Scenario {
    /// 0..=120 — drives multi-page milestone purge (PURGE_BATCH = 8).
    n_milestones: u32,
    /// 0..=40, clamped to n_milestones (disputes need a milestone to target).
    n_disputes: u32,
    /// 0..=3 jury votes filed per dispute.
    dispute_votes: u32,
    /// 0..=20 sub-threshold k-of-n attestation claims left pending.
    n_pending_attestations: u32,
    /// 0..=150 trial offers, each driven to one of the lifecycle states above.
    trial_plan: std::vec::Vec<TrialKind>,
    /// 0..=16 scouts paying to contact the player (distinct contact records).
    n_contacts: u32,
    /// 0..=20 admin `reset_player_level` calls (progress-history length).
    n_resets: u32,
}

fn trial_kind_strategy() -> impl Strategy<Value = TrialKind> {
    prop_oneof![
        Just(TrialKind::Confirm),
        Just(TrialKind::Expire),
        Just(TrialKind::Refund),
        Just(TrialKind::Open),
    ]
}

fn scenario_strategy() -> impl Strategy<Value = Scenario> {
    (
        0u32..=120,
        0u32..=40,
        0u32..=3,
        0u32..=20,
        prop::collection::vec(trial_kind_strategy(), 0..150),
        0u32..=16,
        0u32..=20,
    )
        .prop_map(
            |(n_milestones, n_disputes, dispute_votes, n_pending_attestations, trial_plan, n_contacts, n_resets)| {
                Scenario {
                    n_milestones,
                    // Disputes can only target existing milestones.
                    n_disputes: n_disputes.min(n_milestones),
                    dispute_votes,
                    n_pending_attestations,
                    trial_plan,
                    n_contacts,
                    n_resets,
                }
            },
        )
}

// ── Harness ─────────────────────────────────────────────────────────────────

struct ContractAddresses {
    registration: Address,
    progress: Address,
    verification: Address,
    scout_access: Address,
}

struct Harness {
    env: Env,
    xlm: Address,
    admin: Address,
    registration: RegistrationContractClient<'static>,
    progress: ProgressContractClient<'static>,
    verification: VerificationContractClient<'static>,
    scout_access: ScoutAccessContractClient<'static>,
    addresses: ContractAddresses,
    /// The single player whose erasure is verified. Keeping exactly one
    /// target player makes the oracle unambiguous: every player-keyed entry
    /// in storage belongs to this player.
    player_wallet: Address,
    player_id: u64,
    validators: Vec<Address>,
    scouts: Vec<Address>,
    /// Running ledger timestamp advanced past the trial-offer cooldown.
    now: u64,
    /// Monotonic seed for unique CIDv0 generation.
    cid_seed: u32,
}

impl Harness {
    fn setup(scenario: &Scenario) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        // The test deliberately builds large player histories (up to 120
        // milestones, 150 trial offers, 24 validators, …) that exceed the
        // mainnet-style 100-entry footprint / 50-write per-invocation limits.
        // Disable those limits here, exactly as gas_griefing_regression.rs
        // does for its pagination stress tests. The CPU cost budget is *not*
        // removed — the test still pays for every instruction.
        env.host().set_invocation_resource_limits(None).unwrap();
        // Start well past epoch so expiry/cooldown arithmetic is clean.
        let now: u64 = 10_000_000;
        env.ledger().with_mut(|l| l.timestamp = now);

        let admin = Address::generate(&env);

        // Register all four contracts.
        let reg_id = env.register(RegistrationContract, ());
        let registration = RegistrationContractClient::new(&env, &reg_id);
        registration.initialize(&admin);

        let prog_id = env.register(ProgressContract, ());
        let progress = ProgressContractClient::new(&env, &prog_id);
        progress.initialize(&admin);

        let ver_id = env.register(VerificationContract, ());
        let verification = VerificationContractClient::new(&env, &ver_id);
        verification.initialize(&admin);

        // Native XLM SAC (admin is the issuer).
        let xlm = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let sa_id = env.register(ScoutAccessContract, ());
        let scout_access = ScoutAccessContractClient::new(&env, &sa_id);
        scout_access.initialize(&admin, &xlm, &default_fees());

        // Full cross-contract wiring (mirrors a real deployment):
        //  - verification -> progress : so approve_milestone advances level
        //  - verification -> registration : dispute_milestone wallet check
        //  - progress -> verification : confirm_trial_offer milestone-ref check
        //  - progress -> registration : advance_level syncs PlayerLevel
        //  - progress -> scout_access : confirm_trial_offer secondary caller
        //  - scout_access -> progress : confirm_trial_offer advances level
        //  - registration -> progress : set_player_level caller auth
        verification.set_progress_contract(&prog_id);
        verification.set_registration_contract(&reg_id);
        progress.set_verification_contract(&ver_id);
        progress.set_registration_contract(&reg_id);
        progress.set_scout_access_contract(&sa_id);
        scout_access.set_progress_contract(&prog_id);
        registration.set_progress_contract(&prog_id);

        // Single target player.
        let player_wallet = Address::generate(&env);
        let mut hashes = Vec::new(&env);
        hashes.push_back(String::from_str(&env, "QmPlayerProof"));
        let player_id = registration.register_player(
            &player_wallet,
            &PlayerVitals {
                age: 19,
                position: String::from_str(&env, "Forward"),
                region: String::from_str(&env, "West Africa"),
                nationality: String::from_str(&env, "Ghana"),
            },
            &hashes,
        );

        // Entity pools, sized to the scenario so a 0-milestone case does not
        // pay for 24 validators. Validators: enough for the per-validator
        // milestone cap (5) plus a base of three for disputes / attestations.
        let n_validators = (((scenario.n_milestones + 4) / 5).max(3)).min(24);
        let mut validators = Vec::new(&env);
        for i in 0..n_validators {
            let w = Address::generate(&env);
            verification.register_validator(
                &w,
                &String::from_str(&env, "UEFA-B-License"),
                // Distinct affiliation per validator so PlayerAffiliations grows.
                &String::from_str(&env, &format!("Academy{i}")),
                &Vec::new(&env),
            );
            validators.push_back(w);
        }

        // Scouts: enough for the contact count plus a base of two for the
        // trial-offer rotation.
        let n_scouts = scenario.n_contacts.max(2).min(24);
        let mut scouts = Vec::new(&env);
        for _ in 0..n_scouts {
            let w = Address::generate(&env);
            StellarAssetClient::new(&env, &xlm).mint(&w, &SCOUT_XLM_MINT);
            scout_access.subscribe(&w, &SubscriptionTier::Elite);
            scouts.push_back(w);
        }

        Harness {
            env,
            xlm,
            admin,
            registration,
            progress,
            verification,
            scout_access,
            addresses: ContractAddresses {
                registration: reg_id,
                progress: prog_id,
                verification: ver_id,
                scout_access: sa_id,
            },
            player_wallet,
            player_id,
            validators,
            scouts,
            now,
            cid_seed: 0,
        }
    }

    /// Deterministically mint a syntactically valid CIDv0 (46 chars, "Qm"
    /// prefix) unique across this harness.
    fn next_cid(&mut self) -> String {
        const ALPHABET: &[u8] =
            b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let mut buf = [0u8; 46];
        buf[0] = b'Q';
        buf[1] = b'm';
        self.cid_seed = self.cid_seed.wrapping_add(1);
        let mut x = self.cid_seed as u64;
        for slot in buf.iter_mut().skip(2) {
            *slot = ALPHABET[(x as usize) % ALPHABET.len()];
            x = x.wrapping_mul(131).wrapping_add(7);
        }
        String::from_str(&self.env, core::str::from_utf8(&buf).unwrap())
    }

    /// Advance ledger time to `secs` (monotonic).
    fn advance_time(&mut self, secs: u64) {
        self.now = self.now.saturating_add(secs);
        let ts = self.now;
        self.env.ledger().with_mut(|l| l.timestamp = ts);
    }
}

// ── History builder ─────────────────────────────────────────────────────────

impl Harness {
    /// Phase A: approve `n` milestones with threshold=1 (single-validator
    /// commit). Each approval advances the player's level (capped at Elite)
    /// and creates the full milestone + per-validator + global index surface.
    fn approve_milestones(&mut self, n: u32) {
        for i in 0..n {
            let validator = self.validators.get((i % self.validators.len()) as u32).unwrap();
            let evidence = self.next_cid();
            // Tolerate per-validator cap (5) — round-robin across the pool
            // keeps each validator under the cap for n <= 5 * n_validators.
            let _ = self.verification.try_approve_milestone(
                &validator,
                &self.player_id,
                &String::from_str(&self.env, "goal"),
                &evidence,
                &None,
            );
        }
    }

    /// Phase B: file `n` sub-threshold k-of-n attestation claims that never
    /// commit (threshold raised above the validator count), leaving
    /// `PendingMilestoneClaim` / `PendingMilestoneVote` /
    /// `ValidatorPendingVotes` entries for the purge to remove.
    fn file_pending_attestations(&mut self, n: u32) {
        if self.validators.is_empty() || n == 0 {
            return;
        }
        // Raise the threshold above the number of validators so no claim can
        // reach it (each claim gets a single vote).
        let threshold = self.validators.len() as u32 + 1;
        self.verification.set_milestone_threshold(&threshold);
        for i in 0..n {
            let validator = self.validators.get((i % self.validators.len()) as u32).unwrap();
            let evidence = self.next_cid();
            let _ = self.verification.try_attest_milestone(
                &validator,
                &self.player_id,
                &String::from_str(&self.env, "attested"),
                &evidence,
            );
        }
    }

    /// Open `n` disputes on existing milestones, then file `votes` jury votes
    /// on each (non-jury disputes: impact below the jury threshold so no
    /// voting window applies).
    fn file_disputes(&mut self, n: u32, votes: u32) {
        // Non-jury: impact_score 0 < jury impact_threshold (default 100).
        for d in 0..n {
            // Dispute milestone index (d % n_milestones)+1; safe because the
            // caller clamps n <= n_milestones and milestones are 1-indexed.
            let milestone_index = (d % n.max(1)) + 1;
            let _ = self.verification.try_dispute_milestone(
                &self.player_wallet,
                &self.player_id,
                &milestone_index,
                &String::from_str(&self.env, "disagreement"),
                &0u32,
            );
            for v in 0..votes.min(self.validators.len() as u32) {
                let validator = self.validators.get(v).unwrap();
                let _ = self.verification.try_cast_dispute_vote(
                    &validator,
                    &self.player_id,
                    &milestone_index,
                    &true,
                );
            }
        }
    }

    /// File the trial-offer plan, driving each offer into its assigned
    /// lifecycle state. Ledger time is advanced past the per-(scout, player)
    /// cooldown before each log so a two-scout rotation can file many offers.
    fn file_trial_offers(&mut self, plan: &[TrialKind]) {
        if self.scouts.len() < 2 {
            return;
        }
        for (i, kind) in plan.iter().enumerate() {
            // Cooldown is per (scout, player); advancing time past it lets the
            // alternating scout file again.
            self.advance_time(TRIAL_COOLDOWN_SECS + 1);
            let scout = self.scouts.get((i % 2) as u32).unwrap();
            let details = self.next_cid();
            let res = self
                .scout_access
                .try_log_trial_offer(&scout, &self.player_id, &details);
            let Ok(Ok(index)) = res else {
                // Insufficient balance / cap — skip this offer's lifecycle.
                continue;
            };
            match kind {
                TrialKind::Confirm => {
                    // Confirm in-window (no further time bump). May fail if
                    // the player is already at the top tier; that still
                    // exercises the path and leaves the escrow outstanding.
                    let _ = self.scout_access.try_confirm_trial_offer(
                        &self.player_wallet,
                        &self.player_id,
                        &index,
                        &None::<String>,
                    );
                }
                TrialKind::Expire => {
                    // Past the 2 h expiry window, then sweep via expire.
                    self.advance_time(TRIAL_EXPIRY_SECS + 1);
                    let _ = self.scout_access.try_expire_trial_offers(&20u32);
                }
                TrialKind::Refund => {
                    let _ = self.scout_access.try_admin_refund_trial_escrow(
                        &self.player_id,
                        &index,
                        &scout,
                    );
                }
                TrialKind::Open => {
                    // Left outstanding — purge refunds the escrow.
                }
            }
        }
    }

    /// `n` distinct scouts pay to contact the player (each creates a
    /// `ContactRecord`, `PlayerContacts` entry, `EvidenceAccessGrant` and the
    /// grant pagination index).
    fn file_contacts(&mut self, n: u32) {
        for i in 0..n.min(self.scouts.len() as u32) {
            let scout = self.scouts.get(i).unwrap();
            let _ = self.scout_access.try_pay_to_contact(&scout, &self.player_id);
        }
    }

    /// `n` admin resets of the player's level back to Unverified. Each
    /// records a history entry (milestone_ref = 0) and re-syncs the
    /// registration index, growing the progress-history surface the purge
    /// must clear.
    fn file_resets(&mut self, n: u32) {
        for _ in 0..n {
            let _ = self.progress.try_reset_player_level(
                &self.player_id,
                &ProgressLevel::Unverified,
            );
        }
    }
}

// ── Erasure procedure driver ────────────────────────────────────────────────
//
// Follows docs/PLAYER_ERASURE.md exactly: loop `purge_player_data` feeding
// `next_cursor` back in until `more` is false, for each contract. Recommended
// order: scout_access first (refunds outstanding trial escrows), then
// verification, progress, registration.

fn purge_to_completion(h: &Harness) {
    purge_contract_loop(&h.scout_access, h.player_id, "scout_access");
    purge_contract_loop(&h.verification, h.player_id, "verification");
    purge_contract_loop(&h.progress, h.player_id, "progress");
    purge_contract_loop(&h.registration, h.player_id, "registration");
}

/// Loop `purge_player_data` for one contract, feeding `next_cursor` back in
/// until `more` is `false` — exactly as docs/PLAYER_ERASURE.md prescribes.
/// Generic over the four contract clients via [`PurgeClientLike`].
fn purge_contract_loop<T: PurgeClientLike>(client: &T, player_id: u64, label: &str) {
    let mut cursor = 0u32;
    for _ in 0..100_000 {
        let page = client.purge(&player_id, &cursor);
        cursor = page.next_cursor;
        if !page.more {
            return;
        }
    }
    panic!("{label} purge did not terminate for player {player_id}");
}

/// Unifies the four contract clients' `purge_player_data` so the driver can
/// loop generically without four copy-pasted loops.
trait PurgeClientLike {
    fn purge(&self, player_id: &u64, cursor: &u32) -> scoutchain_shared_types::PurgePage;
}

impl PurgeClientLike for ScoutAccessContractClient<'_> {
    fn purge(&self, player_id: &u64, cursor: &u32) -> scoutchain_shared_types::PurgePage {
        self.purge_player_data(player_id, cursor)
    }
}
impl PurgeClientLike for VerificationContractClient<'_> {
    fn purge(&self, player_id: &u64, cursor: &u32) -> scoutchain_shared_types::PurgePage {
        self.purge_player_data(player_id, cursor)
    }
}
impl PurgeClientLike for ProgressContractClient<'_> {
    fn purge(&self, player_id: &u64, cursor: &u32) -> scoutchain_shared_types::PurgePage {
        self.purge_player_data(player_id, cursor)
    }
}
impl PurgeClientLike for RegistrationContractClient<'_> {
    fn purge(&self, player_id: &u64, cursor: &u32) -> scoutchain_shared_types::PurgePage {
        self.purge_player_data(player_id, cursor)
    }
}

// ── Token-balance conservation ───────────────────────────────────────────────
//
// The purge only ever transfers XLM (refunds outstanding trial escrows back to
// the originating scout); it never mints or burns. So the total XLM held across
// every participant wallet plus the scout_access contract account is invariant
// across the procedure.

fn total_xlm(h: &Harness) -> i128 {
    let token = token::Client::new(&h.env, &h.xlm);
    let mut sum: i128 = token.balance(&h.admin);
    sum = sum.saturating_add(token.balance(&h.player_wallet));
    for i in 0..h.scouts.len() {
        sum = sum.saturating_add(token.balance(&h.scouts.get(i).unwrap()));
    }
    for i in 0..h.validators.len() {
        sum = sum.saturating_add(token.balance(&h.validators.get(i).unwrap()));
    }
    // The scout_access contract account holds escrowed XLM and collected fees.
    sum = sum.saturating_add(token.balance(&h.addresses.scout_access));
    sum
}

// ── The property test ────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 8,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn erasure_completeness_oracle(scenario in scenario_strategy()) {
        let mut h = Harness::setup(&scenario);

        // 1. Build a randomised player history across all four contracts.
        h.approve_milestones(scenario.n_milestones);
        h.file_pending_attestations(scenario.n_pending_attestations);
        h.file_disputes(scenario.n_disputes, scenario.dispute_votes);
        // A reset to Unverified before trial offers lets confirmTrialOffer
        // actually advance the level (the player is likely at Elite after
        // milestone approvals) and exercises the progress-history path.
        if scenario.n_milestones > 0 {
            let _ = h.progress.try_reset_player_level(
                &h.player_id,
                &ProgressLevel::Unverified,
            );
        }
        h.file_trial_offers(&scenario.trial_plan);
        h.file_contacts(scenario.n_contacts);
        h.file_resets(scenario.n_resets);

        // 2. Snapshot total XLM before the procedure.
        let before = total_xlm(&h);

        // 3. Run the documented erasure procedure to completion.
        purge_to_completion(&h);

        // 4. Token-balance conservation across the procedure.
        let after = total_xlm(&h);
        prop_assert_eq!(
            before, after,
            "token-balance conservation violated across the purge procedure \
             (before={}, after={}); scenario={:?}",
            before, after, scenario,
        );

        // 5. Completeness oracle: enumerate REAL storage of every contract,
        //    decode each key, and fail if any non-retained key embeds the
        //    erased player_id.
        let leaks = find_leaks(&h.env, &h.addresses, h.player_id);
        prop_assert!(
            leaks.is_empty(),
            "erasure incomplete: {} player-keyed ledger entr{} left after a full \
             purge (player_id={}): [{}]; scenario={:?}",
            leaks.len(),
            if leaks.len() == 1 { "y" } else { "ies" },
            h.player_id,
            leaks.join(", "),
            scenario,
        );
    }
}

//! Property-based check-precedence table tests for scout_access.
//!
//! For each public function (subscribe, pay_to_contact, batch_contact_players,
//! log_trial_offer) the contract enforces guards in a documented priority order.
//! These tests enumerate **every reachable combination** of the relevant
//! pre-conditions and assert that the actual returned error always matches the
//! highest-priority failing guard — proving the tables hold under all states.
//!
//! No proptest crate is required: we use soroban testutils + nested loops to
//! generate the full combinatorial space, which is equivalent for finite
//! boolean/enum domains.

use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, String,
};

// ── constants ────────────────────────────────────────────────────────────────

const CONTACT_FEE: i128 = 100_000;
const BASIC_FEE: i128 = 1_000_000;
const PRO_FEE: i128 = 3_000_000;
const ELITE_FEE: i128 = 7_000_000;
const SUB_DURATION: u64 = 30 * 24 * 3600; // 30 days
const PRO_LIMIT: u32 = 10;
const START_TIME: u64 = 10_000_000;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: CONTACT_FEE,
        basic_sub_stroops: BASIC_FEE,
        pro_sub_stroops: PRO_FEE,
        elite_sub_stroops: ELITE_FEE,
        sub_duration_secs: SUB_DURATION,
        pro_contact_limit: PRO_LIMIT,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 3_600,
    }
}

// ── shared harness ───────────────────────────────────────────────────────────

struct Harness {
    env: Env,
    xlm: Address,
    contract: ScoutAccessContractClient<'static>,
}

/// Build a harness where the contract is already initialized and NOT paused.
fn setup_initialized() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START_TIME);

    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let id = env.register(ScoutAccessContract, ());
    let contract = ScoutAccessContractClient::new(&env, &id);
    contract.initialize(&admin, &xlm, &default_fees());

    Harness { env, xlm, contract }
}

/// Build a harness where initialize has NOT been called.
fn setup_uninitialized() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START_TIME);

    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let id = env.register(ScoutAccessContract, ());
    let contract = ScoutAccessContractClient::new(&env, &id);
    // deliberately skip initialize

    Harness { env, xlm, contract }
}

/// Pause an already-initialized harness.
fn pause(h: &Harness) {
    h.contract.pause_contract();
}

/// Mint `amount` XLM and subscribe `scout` to `tier`.
fn subscribe(h: &Harness, scout: &Address, tier: &SubscriptionTier) {
    let fee = match tier {
        SubscriptionTier::Basic => BASIC_FEE,
        SubscriptionTier::Pro => PRO_FEE,
        SubscriptionTier::Elite => ELITE_FEE,
    };
    StellarAssetClient::new(&h.env, &h.xlm).mint(scout, &(fee * 2));
    h.contract.subscribe(scout, tier);
}

/// Advance ledger time past subscription expiry.
fn expire_subscription(h: &Harness) {
    h.env.ledger().with_mut(|l| l.timestamp += SUB_DURATION + 1);
}

/// Give a scout enough XLM for many operations.
fn fund(h: &Harness, addr: &Address) {
    StellarAssetClient::new(&h.env, &h.xlm).mint(addr, &100_000_000i128);
}

/// A valid 46-char CIDv0 (no ambiguous chars: no 0, O, I, l).
fn valid_cid(env: &Env) -> String {
    // Exactly 46 chars, starts with Qm, base58btc charset only
    String::from_str(env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG")
}

// ═══════════════════════════════════════════════════════════════════════════
// subscribe — check-precedence table
//
// Priority 1: paused            → ContractPaused
// Priority 2: not initialized   → NotInitialized
// Priority 3: active sub exists + downgrade → SubscriptionDowngradeNotAllowed
// Priority 4: active sub exists + upgrade too soon → UpgradeTooSoon
// ═══════════════════════════════════════════════════════════════════════════

/// Enumerate all (is_paused, is_initialized, has_active_sub, is_downgrade, is_too_soon)
/// triples and assert the first failing guard wins.
#[test]
fn test_subscribe_check_precedence_exhaustive() {
    use scoutchain_scout_access::ScoutAccessError;

    // We test the 3 meaningful downgrade/upgrade scenarios when a sub exists:
    //   downgrade=true  → SubscriptionDowngradeNotAllowed (before UpgradeTooSoon check)
    //   too_soon=true   → UpgradeTooSoon
    //   neither        → success (same tier re-subscribe after expiry or upgrade after interval)
    let sub_scenarios: &[(bool, bool, &str)] = &[
        (true, false, "downgrade"),
        (false, true, "too_soon"),
        (false, false, "ok_upgrade"),
    ];

    for &is_paused in &[true, false] {
        for &is_initialized in &[true, false] {
            for &has_active_sub in &[true, false] {
                for &(is_downgrade, is_too_soon, _label) in sub_scenarios {
                    // Only test sub scenarios when a sub actually exists
                    if !has_active_sub && (is_downgrade || is_too_soon) {
                        continue;
                    }

                    // Determine expected outcome (first failing guard)
                    let expected: Option<ScoutAccessError> = if is_paused {
                        Some(ScoutAccessError::ContractPaused)
                    } else if !is_initialized {
                        Some(ScoutAccessError::NotInitialized)
                    } else if has_active_sub && is_downgrade {
                        Some(ScoutAccessError::SubscriptionDowngradeNotAllowed)
                    } else if has_active_sub && is_too_soon {
                        Some(ScoutAccessError::UpgradeTooSoon)
                    } else {
                        None // success
                    };

                    // Build state
                    let h = if is_initialized {
                        setup_initialized()
                    } else {
                        setup_uninitialized()
                    };

                    let scout = Address::generate(&h.env);

                    if is_initialized && has_active_sub {
                        // Subscribe at Pro first so we can test downgrade (to Basic)
                        // or same-tier upgrade (to Elite, which counts as too_soon if within 1h)
                        subscribe(&h, &scout, &SubscriptionTier::Pro);
                    }

                    // For the "ok upgrade" scenario: advance time past the 1-hour
                    // minimum interval so the upgrade is not rejected as UpgradeTooSoon.
                    if has_active_sub && !is_downgrade && !is_too_soon {
                        h.env.ledger().with_mut(|l| l.timestamp += 3_601);
                    }

                    if is_paused {
                        if !is_initialized {
                            // paused + uninitialized is unreachable in practice
                            continue;
                        }
                        pause(&h);
                    }

                    fund(&h, &scout);

                    let call_tier = if is_downgrade {
                        SubscriptionTier::Basic // downgrade from Pro
                    } else {
                        SubscriptionTier::Elite // upgrade from Pro, or fresh sub
                    };

                    let result = h.contract.try_subscribe(&scout, &call_tier);

                    match expected {
                        None => {
                            assert!(
                                result.is_ok(),
                                "subscribe should succeed: paused={is_paused} init={is_initialized} \
                                 has_sub={has_active_sub} downgrade={is_downgrade} too_soon={is_too_soon}, \
                                 got {result:?}"
                            );
                        }
                        Some(exp_err) => {
                            let actual_err = result
                                .expect_err("expected error")
                                .expect("expected contract error");
                            assert_eq!(
                                actual_err, exp_err,
                                "subscribe precedence wrong: paused={is_paused} init={is_initialized} \
                                 has_sub={has_active_sub} downgrade={is_downgrade} too_soon={is_too_soon}"
                            );
                        }
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// pay_to_contact — check-precedence table
//
// Priority 1: paused              → ContractPaused
// Priority 2: not initialized     → NotInitialized
// Priority 2.5: pay_to_contact paused (function-scoped) → PayToContactPaused
// Priority 3: no subscription     → ScoutNotSubscribed
// Priority 4: subscription expired→ SubscriptionExpired
// Priority 5: already contacted   → AlreadyContacted
// Priority 6: pro quota exceeded  → ProContactLimitReached
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_pay_to_contact_check_precedence_exhaustive() {
    use scoutchain_scout_access::ScoutAccessError;

    // Tier options: None (no sub), Basic, Pro, Elite
    #[derive(Clone, Copy, Debug)]
    enum TierOpt {
        None,
        Basic,
        Pro,
        Elite,
    }

    let tiers = [TierOpt::None, TierOpt::Basic, TierOpt::Pro, TierOpt::Elite];

    for &is_paused in &[true, false] {
        for &is_function_paused in &[true, false] {
            for &is_initialized in &[true, false] {
                for &tier in &tiers {
                    for &is_expired in &[true, false] {
                        for &quota_exceeded in &[true, false] {
                            for &already_contacted in &[true, false] {
                                // Expired only makes sense when sub exists
                                if matches!(tier, TierOpt::None) && is_expired {
                                    continue;
                                }
                                // Quota only applies to Pro tier
                                if !matches!(tier, TierOpt::Pro) && quota_exceeded {
                                    continue;
                                }
                                // Function-scoped pause only reachable when initialized
                                if is_function_paused && !is_initialized {
                                    continue;
                                }

                                let expected: Option<ScoutAccessError> = if is_paused {
                                    Some(ScoutAccessError::ContractPaused)
                                } else if !is_initialized {
                                    Some(ScoutAccessError::NotInitialized)
                                } else if is_function_paused {
                                    Some(ScoutAccessError::PayToContactPaused)
                                } else if matches!(tier, TierOpt::None) {
                                    Some(ScoutAccessError::ScoutNotSubscribed)
                                } else if is_expired {
                                    Some(ScoutAccessError::SubscriptionExpired)
                                } else if already_contacted && !quota_exceeded {
                                    // Reachable only when the quota-exhaustion setup
                                    // did not run: player 1 was actually contacted.
                                    Some(ScoutAccessError::AlreadyContacted)
                                } else if quota_exceeded {
                                    // Quota setup contacts players 100..109, so player 1
                                    // is fresh here; the contact check passes and the
                                    // renewal-aware Pro quota guard fires. The contract
                                    // enforces it with `ProContactLimitReached` and
                                    // checks it *after* the already-contacted guard.
                                    Some(ScoutAccessError::ProContactLimitReached)
                                } else {
                                    None
                                };

                                let h = if is_initialized {
                                    setup_initialized()
                                } else {
                                    setup_uninitialized()
                                };
                                let scout = Address::generate(&h.env);
                                let player_id: u64 = 1;

                                if is_initialized {
                                    // Set up subscription
                                    let sub_tier = match tier {
                                        TierOpt::None => None,
                                        TierOpt::Basic => Some(SubscriptionTier::Basic),
                                        TierOpt::Pro => Some(SubscriptionTier::Pro),
                                        TierOpt::Elite => Some(SubscriptionTier::Elite),
                                    };
                                    if let Some(t) = sub_tier {
                                        subscribe(&h, &scout, &t);
                                    }

                                    if is_expired {
                                        expire_subscription(&h);
                                    }

                                    // Exhaust Pro quota by contacting PRO_LIMIT distinct players
                                    if quota_exceeded {
                                        for pid in 100u64..100 + PRO_LIMIT as u64 {
                                            fund(&h, &scout);
                                            let _ = h.contract.try_pay_to_contact(&scout, &pid);
                                        }
                                    }

                                    if already_contacted
                                        && !quota_exceeded
                                        && !is_expired
                                        && !matches!(tier, TierOpt::None)
                                    {
                                        fund(&h, &scout);
                                        let _ = h.contract.try_pay_to_contact(&scout, &player_id);
                                    }
                                }

                                if is_paused && is_initialized {
                                    pause(&h);
                                } else if is_paused && !is_initialized {
                                    continue; // unreachable state
                                }

                                if is_function_paused {
                                    h.contract.pause_pay_to_contact();
                                }

                                fund(&h, &scout);
                                let result = h.contract.try_pay_to_contact(&scout, &player_id);

                                match expected {
                                None => assert!(
                                    result.is_ok(),
                                    "pay_to_contact should succeed: paused={is_paused} fn_paused={is_function_paused} init={is_initialized} \
                                     tier={tier:?} expired={is_expired} quota={quota_exceeded} contacted={already_contacted}, \
                                     got {result:?}"
                                ),
                                Some(exp) => {
                                    let actual = result.expect_err("expected error").expect("contract error");
                                    assert_eq!(
                                        actual, exp,
                                        "pay_to_contact precedence wrong: paused={is_paused} fn_paused={is_function_paused} init={is_initialized} \
                                         tier={tier:?} expired={is_expired} quota={quota_exceeded} contacted={already_contacted}"
                                    );
                                }
                            }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// batch_contact_players — check-precedence table
//
// Priority 1: paused          → ContractPaused
// Priority 2: not initialized → NotInitialized
// Priority 3: no subscription → ScoutNotSubscribed
// Priority 4: expired         → SubscriptionExpired
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_batch_contact_players_check_precedence_exhaustive() {
    use scoutchain_scout_access::ScoutAccessError;

    #[derive(Clone, Copy, Debug)]
    enum TierOpt {
        None,
        Basic,
        Pro,
        Elite,
    }
    let tiers = [TierOpt::None, TierOpt::Basic, TierOpt::Pro, TierOpt::Elite];

    for &is_paused in &[true, false] {
        for &is_initialized in &[true, false] {
            for &tier in &tiers {
                for &is_expired in &[true, false] {
                    if matches!(tier, TierOpt::None) && is_expired {
                        continue;
                    }

                    let expected: Option<ScoutAccessError> = if is_paused {
                        Some(ScoutAccessError::ContractPaused)
                    } else if !is_initialized {
                        Some(ScoutAccessError::NotInitialized)
                    } else if matches!(tier, TierOpt::None) {
                        Some(ScoutAccessError::ScoutNotSubscribed)
                    } else if is_expired {
                        Some(ScoutAccessError::SubscriptionExpired)
                    } else {
                        None
                    };

                    let h = if is_initialized {
                        setup_initialized()
                    } else {
                        setup_uninitialized()
                    };
                    let scout = Address::generate(&h.env);

                    if is_initialized {
                        let sub_tier = match tier {
                            TierOpt::None => None,
                            TierOpt::Basic => Some(SubscriptionTier::Basic),
                            TierOpt::Pro => Some(SubscriptionTier::Pro),
                            TierOpt::Elite => Some(SubscriptionTier::Elite),
                        };
                        if let Some(t) = sub_tier {
                            subscribe(&h, &scout, &t);
                        }
                        if is_expired {
                            expire_subscription(&h);
                        }
                    }

                    if is_paused && is_initialized {
                        pause(&h);
                    } else if is_paused && !is_initialized {
                        continue;
                    }

                    fund(&h, &scout);
                    let ids = soroban_sdk::vec![&h.env, 1u64, 2u64, 3u64];
                    let result = h.contract.try_batch_contact_players(&scout, &ids);

                    match expected {
                        None => assert!(
                            result.is_ok(),
                            "batch_contact_players should succeed: paused={is_paused} init={is_initialized} \
                             tier={tier:?} expired={is_expired}, got {result:?}"
                        ),
                        Some(exp) => {
                            let actual = result.expect_err("expected error").expect("contract error");
                            assert_eq!(
                                actual, exp,
                                "batch_contact_players precedence wrong: paused={is_paused} init={is_initialized} \
                                 tier={tier:?} expired={is_expired}"
                            );
                        }
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// log_trial_offer — check-precedence table
//
// Priority 1: paused              → ContractPaused
// Priority 2: not initialized     → NotInitialized
// Priority 3: no subscription     → ScoutNotSubscribed
// Priority 4: subscription expired→ SubscriptionExpired
// Priority 5: non-Elite tier      → Unauthorized
// Priority 6: rate limited        → TrialOfferRateLimited
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_log_trial_offer_check_precedence_exhaustive() {
    use scoutchain_scout_access::ScoutAccessError;

    #[derive(Clone, Copy, Debug)]
    enum TierOpt {
        None,
        Basic,
        Pro,
        Elite,
    }
    let tiers = [TierOpt::None, TierOpt::Basic, TierOpt::Pro, TierOpt::Elite];

    for &is_paused in &[true, false] {
        for &tier in &tiers {
            for &is_expired in &[true, false] {
                for &is_rate_limited in &[true, false] {
                    // Expired only meaningful when sub exists
                    if matches!(tier, TierOpt::None) && is_expired {
                        continue;
                    }
                    // Rate-limit only meaningful for Elite (others fail earlier)
                    if !matches!(tier, TierOpt::Elite) && is_rate_limited {
                        continue;
                    }

                    let expected: Option<ScoutAccessError> = if is_paused {
                        Some(ScoutAccessError::ContractPaused)
                    } else if matches!(tier, TierOpt::None) {
                        Some(ScoutAccessError::ScoutNotSubscribed)
                    } else if is_expired {
                        Some(ScoutAccessError::SubscriptionExpired)
                    } else if !matches!(tier, TierOpt::Elite) {
                        Some(ScoutAccessError::Unauthorized)
                    } else if is_rate_limited {
                        Some(ScoutAccessError::TrialOfferRateLimited)
                    } else {
                        None
                    };

                    let h = setup_initialized();
                    let scout = Address::generate(&h.env);
                    let player_id: u64 = 42;

                    let sub_tier = match tier {
                        TierOpt::None => None,
                        TierOpt::Basic => Some(SubscriptionTier::Basic),
                        TierOpt::Pro => Some(SubscriptionTier::Pro),
                        TierOpt::Elite => Some(SubscriptionTier::Elite),
                    };
                    if let Some(t) = sub_tier {
                        subscribe(&h, &scout, &t);
                    }

                    // log_trial_offer requires the scout to have previously
                    // contacted the player (see `ContactRecord` check), so an
                    // Elite scout must make a contact before any offer is logged.
                    if matches!(tier, TierOpt::Elite) {
                        fund(&h, &scout);
                        let _ = h.contract.try_pay_to_contact(&scout, &player_id);
                    }

                    if is_expired {
                        expire_subscription(&h);
                    }

                    // log_trial_offer requires a prior contact record; create it
                    // while the subscription is still active.
                    if matches!(tier, TierOpt::Elite) && !is_expired {
                        fund(&h, &scout);
                        let _ = h.contract.try_pay_to_contact(&scout, &player_id);
                    }

                    // To trigger rate limit: send one offer first, stay within 24h window
                    if is_rate_limited {
                        fund(&h, &scout);
                        let _ =
                            h.contract
                                .try_log_trial_offer(&scout, &player_id, &valid_cid(&h.env));
                        // do NOT advance time — next call within 24h should be rate-limited
                    }

                    if is_paused {
                        pause(&h);
                    }

                    fund(&h, &scout);
                    let result =
                        h.contract
                            .try_log_trial_offer(&scout, &player_id, &valid_cid(&h.env));

                    match expected {
                        None => assert!(
                            result.is_ok(),
                            "log_trial_offer should succeed: paused={is_paused} tier={tier:?} \
                             expired={is_expired} rate_limited={is_rate_limited}, got {result:?}"
                        ),
                        Some(exp) => {
                            let actual =
                                result.expect_err("expected error").expect("contract error");
                            assert_eq!(
                                actual, exp,
                                "log_trial_offer precedence wrong: paused={is_paused} tier={tier:?} \
                                 expired={is_expired} rate_limited={is_rate_limited}"
                            );
                        }
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Edge-case spot-checks: verify the tables hold at exact boundary conditions
// ═══════════════════════════════════════════════════════════════════════════

/// Paused beats NotInitialized: a paused+uninitialized contract is
/// unreachable (you can't pause before initialize), but ContractPaused
/// is checked first in subscribe/pay_to_contact/batch_contact_players.
/// Verify that a freshly initialized+paused contract returns ContractPaused.
#[test]
fn test_paused_beats_subscription_checks() {
    use scoutchain_scout_access::ScoutAccessError;

    let h = setup_initialized();
    let scout = Address::generate(&h.env);
    // Scout has NO subscription and contract IS paused
    pause(&h);

    // subscribe
    let r = h.contract.try_subscribe(&scout, &SubscriptionTier::Elite);
    assert_eq!(
        r.expect_err("err").expect("contract err"),
        ScoutAccessError::ContractPaused
    );

    // pay_to_contact
    let r = h.contract.try_pay_to_contact(&scout, &1u64);
    assert_eq!(
        r.expect_err("err").expect("contract err"),
        ScoutAccessError::ContractPaused
    );

    // batch_contact_players
    let ids = soroban_sdk::vec![&h.env, 1u64];
    let r = h.contract.try_batch_contact_players(&scout, &ids);
    assert_eq!(
        r.expect_err("err").expect("contract err"),
        ScoutAccessError::ContractPaused
    );

    // log_trial_offer
    let r = h
        .contract
        .try_log_trial_offer(&scout, &1u64, &valid_cid(&h.env));
    assert_eq!(
        r.expect_err("err").expect("contract err"),
        ScoutAccessError::ContractPaused
    );
}

/// SubscriptionExpired beats AlreadyContacted — an expired scout who
/// previously contacted a player gets SubscriptionExpired, not AlreadyContacted.
#[test]
fn test_expired_beats_already_contacted() {
    use scoutchain_scout_access::ScoutAccessError;

    let h = setup_initialized();
    let scout = Address::generate(&h.env);
    let player_id: u64 = 7;

    subscribe(&h, &scout, &SubscriptionTier::Elite);
    fund(&h, &scout);
    h.contract.pay_to_contact(&scout, &player_id); // first contact — succeeds

    expire_subscription(&h);

    fund(&h, &scout);
    let r = h.contract.try_pay_to_contact(&scout, &player_id);
    assert_eq!(
        r.expect_err("err").expect("contract err"),
        ScoutAccessError::SubscriptionExpired,
        "SubscriptionExpired should beat AlreadyContacted"
    );
}

/// Non-Elite tier beats TrialOfferRateLimited for log_trial_offer.
#[test]
fn test_non_elite_beats_rate_limit() {
    use scoutchain_scout_access::ScoutAccessError;

    // Pro scout, never sent an offer — should fail with Unauthorized (non-Elite),
    // not TrialOfferRateLimited.
    let h = setup_initialized();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout, &SubscriptionTier::Pro);
    fund(&h, &scout);

    let r = h
        .contract
        .try_log_trial_offer(&scout, &1u64, &valid_cid(&h.env));
    assert_eq!(
        r.expect_err("err").expect("contract err"),
        ScoutAccessError::Unauthorized,
        "non-Elite should return Unauthorized, not TrialOfferRateLimited"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression test: #840 — log_trial_offer must return NotInitialized (not
// ScoutNotSubscribed) when called on an uninitialized contract.
//
// Before the fix, log_trial_offer did not call require_initialized, so the
// first guard it hit on an uninitialized contract was the subscription lookup
// (require_active_subscription), which returned ScoutNotSubscribed because no
// storage had ever been written.  The fix adds require_initialized immediately
// after require_not_paused, matching the ordering of subscribe, pay_to_contact,
// and batch_contact_players.
// ─────────────────────────────────────────────────────────────────────────────

/// Uninitialized contract must return NotInitialized from log_trial_offer,
/// not the misleading ScoutNotSubscribed it returned before issue #840 was fixed.
#[test]
fn test_log_trial_offer_returns_not_initialized_before_initialize() {
    use scoutchain_scout_access::ScoutAccessError;

    let h = setup_uninitialized();
    let scout = Address::generate(&h.env);

    let result = h
        .contract
        .try_log_trial_offer(&scout, &1u64, &valid_cid(&h.env));

    assert_eq!(
        result.expect_err("expected error").expect("contract error"),
        ScoutAccessError::NotInitialized,
        "log_trial_offer on an uninitialized contract must return NotInitialized, \
         not ScoutNotSubscribed — regression guard for issue #840"
    );
}

/// Confirm the fix doesn't break the normal (initialized) path: log_trial_offer
/// still returns ScoutNotSubscribed when the contract IS initialized but the
/// scout has no subscription (the pre-existing, expected behavior).
#[test]
fn test_log_trial_offer_returns_scout_not_subscribed_when_initialized_no_sub() {
    use scoutchain_scout_access::ScoutAccessError;

    let h = setup_initialized();
    let scout = Address::generate(&h.env);

    let result = h
        .contract
        .try_log_trial_offer(&scout, &1u64, &valid_cid(&h.env));

    assert_eq!(
        result.expect_err("expected error").expect("contract error"),
        ScoutAccessError::ScoutNotSubscribed,
        "initialized contract with no subscription should still return ScoutNotSubscribed"
    );
}

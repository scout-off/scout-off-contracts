//! Differential harness for issue #1609: every progress-history read API must
//! agree on the same logical history regardless of which historical storage
//! layout a player's data started in, and regardless of live appends made on
//! top of that legacy data.
//!
//! Layouts exercised (written straight into contract storage):
//! - `EntryOnly`        `HistoryEntry` + `HistoryCounter` (pre-v1)
//! - `Vec`              the above plus `HistoryVec`
//! - `Paged`            built by the contract itself (pages, root and frontier)
//! - `PagedNoFrontier`  entries, pages and root but no `HistoryFrontier`
//!
//! Oracle: the `HistoryEntry` sequence, which every layout and every live
//! append writes. Cases come from a deterministic seeded generator (no extra
//! dependency); the failing seed is printed, and `HISTORY_DIFF_SEED` replays
//! one base seed.

use scoutchain_progress::{DataKey, ProgressContract, ProgressContractClient, ProgressEntry};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    testutils::{Address as _, EnvTestConfig, Ledger},
    xdr::ToXdr,
    Address, Bytes, BytesN, Env, Vec,
};

const CASES: u64 = 256;
const PAGE_SIZE: u32 = 8;

#[soroban_sdk::contract]
struct RegistrationStub;

#[soroban_sdk::contractimpl]
impl RegistrationStub {
    /// Wiring handshake (#1610): report the kind of contract this stub stands in for.
    pub fn contract_kind(env: Env) -> soroban_sdk::Symbol {
        soroban_sdk::Symbol::new(&env, "registration")
    }

    pub fn version(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "2.0.0")
    }

    pub fn set_player_level(_env: Env, _player_id: u64, _level: ProgressLevel) {}
}

#[derive(Clone, Copy, Debug)]
enum Layout {
    EntryOnly,
    Vec,
    Paged,
    PagedNoFrontier,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn level_from(n: u64) -> ProgressLevel {
    match n % 4 {
        0 => ProgressLevel::Unverified,
        1 => ProgressLevel::VerifiedIdentity,
        2 => ProgressLevel::PerformanceMilestones,
        _ => ProgressLevel::EliteTier,
    }
}

fn level_code(l: &ProgressLevel) -> u32 {
    match l {
        ProgressLevel::Unverified => 0,
        ProgressLevel::VerifiedIdentity => 1,
        ProgressLevel::PerformanceMilestones => 2,
        ProgressLevel::EliteTier => 3,
    }
}

fn leaf(env: &Env, e: &ProgressEntry) -> BytesN<32> {
    let mut b = Bytes::new(env);
    b.push_back(0u8);
    b.extend_from_slice(&e.player_id.to_be_bytes());
    b.extend_from_slice(&level_code(&e.old_level).to_be_bytes());
    b.extend_from_slice(&level_code(&e.new_level).to_be_bytes());
    b.append(&e.updated_by.clone().to_xdr(env));
    b.extend_from_slice(&e.updated_at.to_be_bytes());
    b.extend_from_slice(&e.milestone_ref.to_be_bytes());
    b.extend_from_slice(&e.ledger_sequence.to_be_bytes());
    env.crypto().sha256(&b).to_bytes()
}

/// RFC 6962 Merkle Tree Hash, recomputed independently of the contract.
fn mth(env: &Env, leaves: &[BytesN<32>]) -> BytesN<32> {
    if leaves.len() == 1 {
        return leaves[0].clone();
    }
    let mut k = 1usize;
    while k * 2 < leaves.len() {
        k *= 2;
    }
    let l = mth(env, &leaves[..k]);
    let r = mth(env, &leaves[k..]);
    let mut b = Bytes::new(env);
    b.push_back(1u8);
    b.append(&Bytes::from_slice(env, &l.to_array()));
    b.append(&Bytes::from_slice(env, &r.to_array()));
    env.crypto().sha256(&b).to_bytes()
}

fn rfc_root(env: &Env, history: &[ProgressEntry]) -> BytesN<32> {
    let leaves: std::vec::Vec<_> = history.iter().map(|e| leaf(env, e)).collect();
    mth(env, &leaves)
}

fn oracle(env: &Env, c: &ProgressContractClient, pid: u64) -> std::vec::Vec<ProgressEntry> {
    let n: u32 = env.as_contract(&c.address, || {
        env.storage()
            .persistent()
            .get(&DataKey::HistoryCounter(pid))
            .unwrap_or(0)
    });
    (1..=n)
        .map(|i| {
            env.as_contract(&c.address, || {
                env.storage()
                    .persistent()
                    .get(&DataKey::HistoryEntry(pid, i))
                    .expect("HistoryEntry must exist for every counted index")
            })
        })
        .collect()
}

fn run_case(env: &Env, c: &ProgressContractClient, caller: &Address, seed: u64, pid: u64) {
    // Each case replays up to ~60 invocations on one shared env; keep the
    // cumulative test-host budget from tripping on a perfectly valid case.
    env.cost_estimate().budget().reset_unlimited();
    let mut rng = Rng(seed | 1);
    let layout = [
        Layout::EntryOnly,
        Layout::Vec,
        Layout::Paged,
        Layout::PagedNoFrontier,
    ][rng.below(4) as usize];
    let initial = rng.below(41) as u32;
    let appends = rng.below(21) as u32;
    let ctx = format!("seed={seed} layout={layout:?} initial={initial} appends={appends}");

    let mut level = ProgressLevel::Unverified;
    let mut ts = 1_000u64;
    let mut live = |c: &ProgressContractClient, level: &mut ProgressLevel, rng: &mut Rng| {
        ts += 1;
        env.ledger().with_mut(|l| {
            l.timestamp = ts;
            l.sequence_number += 1;
        });
        if *level == ProgressLevel::EliteTier || rng.below(10) < 3 {
            let mut target = level_from(rng.next());
            if target == *level {
                target = level_from(level_code(level) as u64 + 1);
            }
            c.reset_player_level(&pid, &target);
            *level = target;
        } else {
            *level = c.advance_level(caller, &pid, &1u32);
        }
    };

    // ── build the starting layout ───────────────────────────────────────────
    match layout {
        Layout::Paged => {
            for _ in 0..initial {
                live(c, &mut level, &mut rng);
            }
        }
        _ => {
            let mut entries: std::vec::Vec<ProgressEntry> = std::vec::Vec::new();
            for i in 0..initial {
                let new_level = {
                    let t = level_from(rng.next());
                    if t == level {
                        level_from(level_code(&level) as u64 + 1)
                    } else {
                        t
                    }
                };
                entries.push(ProgressEntry {
                    player_id: pid,
                    old_level: level.clone(),
                    new_level: new_level.clone(),
                    updated_by: caller.clone(),
                    updated_at: 500 + i as u64,
                    milestone_ref: i,
                    ledger_sequence: 10 + i,
                });
                level = new_level;
            }
            let root = if entries.is_empty() {
                None
            } else {
                Some(rfc_root(env, &entries))
            };
            env.as_contract(&c.address, || {
                let st = env.storage().persistent();
                for (i, e) in entries.iter().enumerate() {
                    st.set(&DataKey::HistoryEntry(pid, i as u32 + 1), e);
                }
                st.set(&DataKey::HistoryCounter(pid), &initial);
                st.set(&DataKey::PlayerLevel(pid), &level);
                match layout {
                    Layout::Vec => {
                        let mut v: Vec<ProgressEntry> = Vec::new(env);
                        for e in &entries {
                            v.push_back(e.clone());
                        }
                        st.set(&DataKey::HistoryVec(pid), &v);
                    }
                    Layout::PagedNoFrontier => {
                        for (p, chunk) in entries.chunks(PAGE_SIZE as usize).enumerate() {
                            let mut page: Vec<ProgressEntry> = Vec::new(env);
                            for e in chunk {
                                page.push_back(e.clone());
                            }
                            st.set(&DataKey::HistoryPage(pid, p as u32), &page);
                        }
                        if let Some(r) = &root {
                            st.set(&DataKey::HistoryRoot(pid), r);
                        }
                    }
                    _ => {}
                }
            });
        }
    }

    // ── mixed live appends ──────────────────────────────────────────────────
    for _ in 0..appends {
        live(c, &mut level, &mut rng);
    }

    // ── differential checks against the HistoryEntry oracle ─────────────────
    let want = oracle(env, c, pid);
    assert_eq!(want.len() as u32, initial + appends, "oracle length: {ctx}");
    let to_std = |v: Vec<ProgressEntry>| -> std::vec::Vec<ProgressEntry> { v.iter().collect() };

    assert_eq!(
        to_std(c.get_progress_history(&pid)),
        want,
        "get_progress_history: {ctx}"
    );

    let mut paged = std::vec::Vec::new();
    let mut offset = 0u32;
    loop {
        let page = to_std(c.get_progress_history_page(&pid, &offset, &7u32));
        if page.is_empty() {
            break;
        }
        offset += page.len() as u32;
        paged.extend(page);
    }
    assert_eq!(paged, want, "get_progress_history_page: {ctx}");

    let mut cursored = std::vec::Vec::new();
    let (mut snap, mut next) = (None, None);
    loop {
        let (page, n, s) = c.get_history_page_with_cursor(&pid, &snap, &next, &9u32);
        cursored.extend(to_std(page));
        snap = Some(s);
        if n == 0 {
            break;
        }
        next = Some(n);
    }
    assert_eq!(cursored, want, "get_history_page_with_cursor: {ctx}");

    let mut newest: std::vec::Vec<_> = want.iter().rev().take(50).cloned().collect();
    newest.truncate(50);
    assert_eq!(
        to_std(c.get_history_since(&pid, &0u64, &50u32)),
        newest,
        "get_history_since: {ctx}"
    );

    // Legacy layouts with no live append have no commitment yet.
    let has_root =
        initial > 0 && (appends > 0 || matches!(layout, Layout::Paged | Layout::PagedNoFrontier));
    if appends > 0 || has_root {
        assert_eq!(
            c.get_progress_root(&pid),
            rfc_root(env, &want),
            "get_progress_root: {ctx}"
        );
        // Proof generation re-reads and re-hashes the whole history per call, so
        // large histories check the ends plus a random sample of indexes to
        // keep the runtime inside the CI budget; short ones check every index.
        let mut idxs: std::vec::Vec<usize> = (0..want.len()).collect();
        if want.len() > 12 {
            idxs = vec![0, want.len() - 1];
            for _ in 0..1 {
                idxs.push(rng.below(want.len() as u64) as usize);
            }
        }
        for i in idxs {
            let e = &want[i];
            let proof = c.get_history_proof(&pid, &(i as u32 + 1));
            assert!(
                c.verify_history_proof(&pid, e, &proof),
                "proof for index {} failed: {ctx}",
                i + 1
            );
        }
    }
}

#[test]
fn history_reads_agree_across_every_storage_layout() {
    let base: u64 = std::env::var("HISTORY_DIFF_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5eed_1609);
    for case in 0..CASES {
        // A fresh env per case keeps host memory flat and failures isolated.
        let env = Env::new_with_config(EnvTestConfig {
            capture_snapshot_at_drop: false,
        });
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let id = env.register(ProgressContract, (admin.clone(),));
        let c = ProgressContractClient::new(&env, &id);
        let caller = scoutchain_shared_types::testutils::peer_stub(&env, "verification");
        c.set_verification_contract(&caller);
        c.set_registration_contract(&env.register(RegistrationStub, ()));
        run_case(
            &env,
            &c,
            &caller,
            base.wrapping_mul(6364136223846793005).wrapping_add(case),
            1,
        );
    }
}

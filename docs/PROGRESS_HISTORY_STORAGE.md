# Progress history storage

How the `progress` contract stores a player's level-change history, which keys
are authoritative, how each getter reads them, and what a migration or seed must
write. Source of truth: `record_progress_entry`, `read_history_pages`,
`load_or_build_frontier`, `backfill_history_vec` and `admin_seed_history` in
`contracts/progress/src/lib.rs`, and the `DataKey` docs in
`contracts/progress/src/types.rs`.

## Key families

All keys live in **persistent** storage. `HISTORY_PAGE_SIZE = 8`.

| Key | Value | Role |
|-----|-------|------|
| `HistoryCounter(player)` | `u32` | Number of entries; also the index of the newest entry. **Authoritative** for history length. |
| `HistoryEntry(player, i)` | `ProgressEntry` | One entry, `i` in `1..=count`. **Authoritative** per-entry record; every other copy can be rebuilt from these. |
| `HistoryPage(player, p)` | `Vec<ProgressEntry>` | Bounded shard holding entries `p*8+1 ..= p*8+8`. Read-path cache; written on live appends only. |
| `HistoryRoot(player)` | `BytesN<32>` | RFC 6962 Merkle root over the full history. |
| `HistoryFrontier(player)` | `Vec<FrontierPeak>` | Incremental Merkle accumulator (#1368) so appends cost O(log n) hashes. Rebuilt lazily if absent. |
| `HistoryVec(player)` | `Vec<ProgressEntry>` | **Legacy** unbounded snapshot. Not written on live appends; written by `admin_seed_history` and the `backfill_history_vec` migration; last-resort read fallback. |

## Keys written per append

```
record_progress_entry(player, entry)          # live path (advance_level etc.)
  ├─ HistoryEntry(player, n+1)   = entry
  ├─ HistoryCounter(player)      = n+1
  ├─ HistoryPage(player, n/8)    += entry       # page index = (i-1)/8
  ├─ HistoryFrontier(player)     = fold(frontier, leaf_hash(entry))
  │                                 (load_or_build_frontier rebuilds it from
  │                                  read_history_pages if missing)
  └─ HistoryRoot(player)         = root(frontier)

admin_seed_history(player, i, entry, expected_root?)   # migration replay
  ├─ HistoryEntry(player, i)     = entry
  ├─ HistoryCounter(player)      = i
  ├─ HistoryVec(player)          += entry
  ├─ HistoryFrontier(player)     = build_frontier(HistoryVec)
  └─ HistoryRoot(player)         = root(frontier)   # checked against expected_root
     (no HistoryPage write: readers reconstruct pages from HistoryEntry)
```

## Read path per getter

| Getter | Keys read |
|--------|-----------|
| `get_history_count` | `HistoryCounter` |
| `get_history_entry` | `HistoryEntry` |
| `get_progress_history_page` / `get_history_page_with_cursor` | `HistoryCounter`, `HistoryEntry` |
| `get_history_since` | `HistoryPage`; a missing page is reconstructed from `HistoryEntry` |
| `get_progress_history` / `get_history_proof` | `read_history_pages`: each `HistoryPage`, missing pages rebuilt from `HistoryEntry`; if `HistoryCounter` is 0 or the result is empty, falls back to `HistoryVec` |
| `get_progress_root` | `HistoryRoot` |

## TTL policy per key

Every key above is extended to `PERSISTENT_TTL_MIN = 500` /
`PERSISTENT_TTL_MAX = 518_400` ledgers (~30 days) **when it is written**.
Reads do not bump TTL, so an inactive player's keys can be archived and must be
restored before reading. Because readers fall back across families, an archived
`HistoryPage` degrades to `HistoryEntry` reads rather than to data loss, but an
archived `HistoryEntry` with no page or `HistoryVec` copy is unrecoverable from
on-chain state. See [TTL_POLICY.md](TTL_POLICY.md).

## What a migration or seed must write

- **Required:** `HistoryEntry(player, i)` for every `i` in `1..=count` and
  `HistoryCounter(player) = count`. These are what every getter ultimately
  falls back to.
- **Required for proofs:** `HistoryRoot`, matching the old deployment's root
  (pass it as `expected_root` on the final `admin_seed_history` call).
- **Optional:** `HistoryFrontier` (rebuilt on next append if absent),
  `HistoryPage` (reconstructed from `HistoryEntry` on read), `HistoryVec`
  (legacy fallback only; `backfill_history_vec` fills it from `HistoryEntry`).
- Entries must be seeded in ascending order with no gaps; `admin_seed_history`
  rejects out-of-order indices with `InvalidHistoryIndex`.

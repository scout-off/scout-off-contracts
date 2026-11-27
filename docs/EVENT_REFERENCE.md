# Event Reference

## Player erasure

The `progress`, `verification`, and `scout_access` contracts emit the following
events while `purge_player_data` removes a player's data. Each event's first
topic is its event name; the listed topic and data values follow it.

| Event | Additional topics | Data | Emission |
|-------|-------------------|------|----------|
| `player_purge_started` | `admin` | `player_id: u64` | First purge page for a player |
| `player_data_purged` | `admin` | `(player_id: u64, entries_removed: u32, next_cursor: u32, more: bool)` | Every purge page |
| `player_purge_completed` | `admin` | `player_id: u64` | Final page, when no data remains |

`player_data_purged` reports the persisted next cursor. The `more` field is
`true` when another page remains. For authoritative current status, query
`get_purge_status(player_id)` on the relevant contract; the event history alone
is not required to determine whether a purge completed. See
[PLAYER_ERASURE.md](PLAYER_ERASURE.md) for the operator procedure.

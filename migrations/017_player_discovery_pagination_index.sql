-- migrations/017_player_discovery_pagination_index.sql
-- Adds a composite index on (level DESC, player_id ASC) to support
-- efficient keyset pagination for the player discovery API.
--
-- The API uses keyset pagination ordered by level DESC, player_id ASC.
-- This composite index ensures the pagination queries are efficient even
-- when combined with region/position filters.
--
-- Safe to re-run: CREATE INDEX IF NOT EXISTS is idempotent.

CREATE INDEX IF NOT EXISTS idx_players_level_player_id
  ON players (level DESC, player_id ASC);

COMMENT ON INDEX idx_players_level_player_id IS
  'Supports keyset pagination for player discovery API (level DESC, player_id ASC).';

-- ScoutChain — align players column sizes with on-chain contract limits
--
-- Context:
--   The registration contract enforces:
--     MAX_STRING_LEN = 64   — applied to players.region and players.nationality
--     MAX_REGION_LEN = 128  — applied only to the region filter parameter in
--                             filter_players() (scout discovery query)
--
--   Migration 001 declared both players.region and players.nationality as
--   VARCHAR(128), which is wider than the 64-byte limit the contract enforces.
--   This mismatch means the DB could theoretically store values that the
--   contract would never produce, complicating validation in application code.
--
--   scouts.region VARCHAR(128) correctly matches MAX_REGION_LEN and is unchanged.
--   players.position VARCHAR(64) already matches MAX_STRING_LEN and is unchanged.
--
-- This migration:
--   1. Narrows players.region     from VARCHAR(128) → VARCHAR(64)
--   2. Narrows players.nationality from VARCHAR(128) → VARCHAR(64)
--   3. Adds CHECK constraints on both columns as an additional safety net
--
-- Note: In PostgreSQL, narrowing a VARCHAR column is safe when all existing rows
-- fit within the new limit. If existing data contains values longer than 64 chars
-- the ALTER TABLE will fail — run the query below to check first:
--
--   SELECT COUNT(*) FROM players
--   WHERE length(region) > 64 OR length(nationality) > 64;
--
-- NEVER edit migration 001 DDL — this migration is the correct fix.

BEGIN;

ALTER TABLE players
    ALTER COLUMN region      TYPE VARCHAR(64),
    ALTER COLUMN nationality TYPE VARCHAR(64);

ALTER TABLE players
    ADD CONSTRAINT chk_players_region_len
        CHECK (length(region) <= 64),
    ADD CONSTRAINT chk_players_nationality_len
        CHECK (length(nationality) <= 64);

COMMIT;

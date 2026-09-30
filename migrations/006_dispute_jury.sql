-- Migration 006: Dispute jury escalation system (issue #1036)
--
-- Adds jury-related columns to milestone_disputes and a new dispute_votes
-- table for per-validator audit trail.
--
-- The new milestone_disputes columns default to values that preserve the
-- existing meaning for rows written before this migration:
--   impact_score    = 0   (below the default threshold of 100 → admin path)
--   jury_required   = FALSE
--   quorum          = 0
--   voting_deadline = 0
--   votes_for       = 0
--   votes_against   = 0
--
-- This migration is idempotent: all ADD COLUMN statements use IF NOT EXISTS,
-- and the CREATE TABLE uses IF NOT EXISTS.

-- -----------------------------------------------------------------------
-- milestone_disputes — add jury fields
-- -----------------------------------------------------------------------
ALTER TABLE milestone_disputes
    ADD COLUMN IF NOT EXISTS impact_score     INTEGER  NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS jury_required    BOOLEAN  NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS quorum           INTEGER  NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS voting_deadline  BIGINT   NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS votes_for        INTEGER  NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS votes_against    INTEGER  NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_milestone_disputes_jury
    ON milestone_disputes (jury_required)
    WHERE jury_required = TRUE;

-- -----------------------------------------------------------------------
-- dispute_votes — per-validator audit trail for jury votes
-- (verification.cast_dispute_vote)
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS dispute_votes (
    id              SERIAL       PRIMARY KEY,
    player_id       BIGINT       NOT NULL,
    milestone_index INTEGER      NOT NULL CHECK (milestone_index > 0),
    validator       VARCHAR(56)  NOT NULL,
    for_upheld      BOOLEAN      NOT NULL,
    voted_at        BIGINT       NOT NULL,           -- Unix timestamp
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (player_id, milestone_index, validator)
);

CREATE INDEX IF NOT EXISTS idx_dispute_votes_dispute
    ON dispute_votes (player_id, milestone_index);

CREATE INDEX IF NOT EXISTS idx_dispute_votes_validator
    ON dispute_votes (validator);

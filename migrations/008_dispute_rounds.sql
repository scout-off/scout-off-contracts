-- Migration 008: Dispute rounds (issue #1396)
--
-- Disputes are now keyed by (player_id, milestone_index, round). A resolved
-- dispute may be re-opened after DISPUTE_REOPEN_COOLDOWN_SECS up to
-- MAX_DISPUTE_ROUNDS. Indexers must store round and resolved_at.

ALTER TABLE milestone_disputes
    ADD COLUMN IF NOT EXISTS round        INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS resolved_at_unix BIGINT NOT NULL DEFAULT 0;

-- Replace the old unique key with a round-aware one.
ALTER TABLE milestone_disputes
    DROP CONSTRAINT IF EXISTS milestone_disputes_player_id_milestone_index_key;

ALTER TABLE milestone_disputes
    ADD CONSTRAINT milestone_disputes_player_milestone_round_key
    UNIQUE (player_id, milestone_index, round);

CREATE INDEX IF NOT EXISTS idx_milestone_disputes_round
    ON milestone_disputes (player_id, milestone_index, round);

-- dispute_votes gain a round column so jury votes from prior rounds stay auditable.
ALTER TABLE dispute_votes
    ADD COLUMN IF NOT EXISTS round INTEGER NOT NULL DEFAULT 0;

ALTER TABLE dispute_votes
    DROP CONSTRAINT IF EXISTS dispute_votes_player_id_milestone_index_validator_key;

ALTER TABLE dispute_votes
    ADD CONSTRAINT dispute_votes_player_milestone_round_validator_key
    UNIQUE (player_id, milestone_index, round, validator);

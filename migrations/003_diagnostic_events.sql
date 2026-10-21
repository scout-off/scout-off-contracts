-- migrations/003_diagnostic_events.sql
-- Adds a table for indexing diagnostic (observability) events emitted by the
-- verification and scout_access contracts when cross-contract level advancement
-- is skipped or fails.  These events surface silent failures that would
-- otherwise be invisible to the off-chain indexer.
--
-- Safe to re-run: CREATE TABLE IF NOT EXISTS is idempotent.
--
-- Events indexed by this table:
--
--   level_advancement_skipped  (verification)
--     Emitted when a milestone is recorded but advance_level is not called
--     because the player is already at EliteTier.  The milestone is persisted;
--     the level is not changed.
--     event_type = 'level_advancement_skipped', reason = 'AlreadyAtMaxLevel'
--
--   progress_contract_not_set  (verification, scout_access)
--     Emitted when the progress contract address has not been configured.
--     Indicates missing wiring — should alert in production.
--     event_type = 'progress_contract_not_set', reason = NULL
--
--   progress_call_failed  (verification, scout_access)
--     Emitted just before ProgressCallFailed is returned so the indexer can
--     detect it via receipt scanning.  Because the error aborts the whole
--     transaction this event only appears in the diagnostic stream; it is NOT
--     committed to the ledger.
--     event_type = 'progress_call_failed', error_code = raw discriminant

CREATE TABLE IF NOT EXISTS diagnostic_events (
    id              SERIAL       PRIMARY KEY,
    -- Which contract emitted the event
    contract_name   VARCHAR(32)  NOT NULL CHECK (contract_name IN ('verification', 'scout_access')),
    -- Event discriminant — mirrors the Soroban event topic string
    event_type      VARCHAR(64)  NOT NULL CHECK (
                        event_type IN (
                            'level_advancement_skipped',
                            'progress_contract_not_set',
                            'progress_call_failed'
                        )
                    ),
    player_id       BIGINT       NOT NULL,
    -- For 'level_advancement_skipped': 'AlreadyAtMaxLevel'
    -- For 'progress_contract_not_set': NULL
    -- For 'progress_call_failed': NULL (use error_code instead)
    reason          TEXT,
    -- Numeric error discriminant from the failed cross-contract call.
    -- Populated only for 'progress_call_failed'; NULL otherwise.
    error_code      INTEGER,
    -- Ledger sequence in which the event was observed (for ordering / replay).
    ledger_sequence BIGINT,
    -- Timestamp at which the indexer recorded this row.
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_diagnostic_events_player
    ON diagnostic_events (player_id);

CREATE INDEX IF NOT EXISTS idx_diagnostic_events_type
    ON diagnostic_events (event_type);

CREATE INDEX IF NOT EXISTS idx_diagnostic_events_contract
    ON diagnostic_events (contract_name);

COMMENT ON TABLE diagnostic_events IS
    'Off-chain audit log of diagnostic events emitted by verification and '
    'scout_access when cross-contract level advancement is silently skipped '
    'or fails.  level_advancement_skipped and progress_contract_not_set are '
    'committed ledger events; progress_call_failed is a diagnostic-only event '
    '(not committed) that appears only in transaction receipts.';

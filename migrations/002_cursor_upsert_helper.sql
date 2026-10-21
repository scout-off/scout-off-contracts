-- migrations/002_cursor_upsert_helper.sql
-- Adds an idiomatic upsert helper for advancing the Horizon event-stream
-- cursor so individual indexer implementations do not each have to hand-write
-- the same ON CONFLICT clause.
--
-- Safe to re-run: CREATE OR REPLACE replaces the function body on subsequent
-- runs without error.
--
-- Usage (from the backend indexer after processing a batch of events):
--
--   SELECT advance_indexer_cursor(42391);
--
-- The function updates last_ledger only when the supplied value is greater
-- than the stored value, preventing a stale replay from accidentally
-- rewinding the cursor.

CREATE OR REPLACE FUNCTION advance_indexer_cursor(p_ledger BIGINT)
RETURNS VOID
LANGUAGE plpgsql
AS $$
BEGIN
    INSERT INTO indexer_cursor (id, last_ledger, updated_at)
    VALUES (1, p_ledger, NOW())
    ON CONFLICT (id) DO UPDATE
        SET last_ledger = EXCLUDED.last_ledger,
            updated_at  = EXCLUDED.updated_at
    WHERE indexer_cursor.last_ledger < EXCLUDED.last_ledger;
END;
$$;

COMMENT ON FUNCTION advance_indexer_cursor(BIGINT) IS
    'Idempotent upsert for the indexer_cursor checkpoint row. '
    'Updates last_ledger only when p_ledger > the stored value, '
    'so replaying an old batch never rewinds the cursor.';

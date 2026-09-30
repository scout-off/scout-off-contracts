-- Migration 008: deactivated_scout — scout deactivation tracking (issue #1403)
--
-- Adds a `deactivated` column to the `scouts` table so the indexer can
-- surface which scouts have been deactivated by the registration admin.
--
-- Idempotent (IF NOT EXISTS) and safe to re-run.

-- ── scouts.deactivated ──────────────────────────────────────────────────────
--
-- `TRUE` when the `ScoutDeactivated(scout_id)` persistent flag is set on the
-- registration contract; `FALSE` (default) when the flag is absent (active).
-- Reconciliation logic checks this column against the live contract state.
ALTER TABLE scouts ADD COLUMN IF NOT EXISTS deactivated BOOLEAN NOT NULL DEFAULT FALSE;
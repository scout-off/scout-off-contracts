-- migrations/016_cleanup_jobs.sql
-- Scheduled cleanup for the three auth-related tables.
--
-- Cleanup policy (all times relative to current server time):
--   sep10_challenges:  delete where expires_at < now() - INTERVAL '1 day'
--   refresh_tokens:    delete where expires_at < now() - INTERVAL '1 day'
--                      OR (revoked_at IS NOT NULL AND revoked_at < now() - INTERVAL '1 day')
--   revoked_tokens:    delete where expires_at < now()
--                      OR (expires_at IS NULL AND created_at + INTERVAL '15 minutes' < now())
--
-- This ensures:
--   - Expired tokens/challenges are removed 1 day after expiry.
--   - Revoked tokens are never deleted before their expiry.
--   - NULL expires_at (old rows) fall back to created_at + max_ttl (15 min).
--
-- The cleanup should run on an interval (e.g., every 5-15 minutes) via
-- cron, Kubernetes CronJob, or a scheduled job in the application.
--
-- Safe to re-run: CREATE INDEX IF NOT EXISTS, ALTER TABLE ADD COLUMN IF NOT EXISTS.

CREATE INDEX IF NOT EXISTS idx_sep10_challenges_expires_at_consumed_at
    ON sep10_challenges (expires_at, consumed_at);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at_revoked_at
    ON refresh_tokens (expires_at, revoked_at);

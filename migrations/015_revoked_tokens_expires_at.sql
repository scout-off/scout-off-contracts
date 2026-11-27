-- migrations/015_revoked_tokens_expires_at.sql
-- Add expires_at to revoked_tokens to enable proper cleanup without
-- accidentally removing valid revocations. Tokens are revoked based on
-- their actual expiry, not the time of revocation.
--
-- Safety:
--   - ALTER TABLE ADD COLUMN is safe to re-run (idempotent).
--   - NULL values fall back to created_at + max_ttl for cleanup.
--   - Existing rows are safe; they won't be cleaned until their
--     created_at + max_ttl window has passed.

ALTER TABLE revoked_tokens
    ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_revoked_tokens_expires_at
    ON revoked_tokens (expires_at);

COMMENT ON COLUMN revoked_tokens.expires_at IS
    'Token expiry timestamp (from JWT exp claim). Used for cleanup: '
    'a revocation must not be deleted before the token would have expired. '
    'NULL falls back to created_at + max_ttl (15 minutes for access tokens).';

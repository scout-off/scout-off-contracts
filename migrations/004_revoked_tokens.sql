-- migrations/004_revoked_tokens.sql
-- Creates the revoked_tokens table required by the backend API to invalidate
-- JWT tokens on explicit sign-out or credential rotation.
--
-- Ownership: this migration is the canonical source of truth for the
-- revoked_tokens table schema.  The Prisma model in prisma/schema.prisma must
-- use @@map("revoked_tokens") to map to this table name.  See
-- migrations/README.md for the full migration ownership policy.
--
-- Safe to re-run: CREATE TABLE IF NOT EXISTS is idempotent.
--
-- Cleanup: The index on created_at supports efficient deletion of expired
-- tokens by background cleanup jobs:
--
--   DELETE FROM revoked_tokens
--   WHERE created_at < NOW() - INTERVAL '7 days';

CREATE TABLE IF NOT EXISTS revoked_tokens (
    -- JWT ID claim (jti) — unique per token; used as the revocation key.
    -- The jti is a UUID string (RFC 7519 §4.1.7) generated at issuance time.
    jti             VARCHAR(128) PRIMARY KEY,

    -- Wall-clock timestamp at which the token was revoked (i.e. when the
    -- revocation record was inserted, not when the JWT was originally issued).
    -- Used by cleanup jobs to purge expired revocation records; indexed below.
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Supports efficient range-based cleanup of old revocation records:
--   DELETE FROM revoked_tokens WHERE created_at < NOW() - INTERVAL '7 days';
CREATE INDEX IF NOT EXISTS idx_revoked_tokens_created_at
    ON revoked_tokens (created_at);

COMMENT ON TABLE revoked_tokens IS
    'Explicit JWT revocation list. Each row records the jti of a token that has '
    'been invalidated before its natural expiry (e.g. on sign-out, password '
    'change, or admin revocation). The backend auth middleware checks this table '
    'on every authenticated request. Rows older than the maximum JWT lifetime '
    'may be deleted safely by a scheduled cleanup job.';

COMMENT ON COLUMN revoked_tokens.jti IS
    'JWT ID claim (RFC 7519 §4.1.7). A UUID generated at token issuance and '
    'stored in the JWT payload. Used as the revocation key.';

COMMENT ON COLUMN revoked_tokens.created_at IS
    'Timestamp at which the revocation record was inserted. Used for cleanup; '
    'see idx_revoked_tokens_created_at.';

-- migrations/014_sep10_sessions.sql
-- SEP-10 session model (#1606): a single-use challenge store and rotating
-- server-side refresh tokens.
--
-- sep10_challenges.hash is sha256(challenge XDR) in hex (64 chars): the full
-- base64 XDR does not fit revoked_tokens.jti VARCHAR(128). A challenge is
-- consumed with an atomic
--   UPDATE sep10_challenges SET consumed_at = NOW()
--   WHERE hash = $1 AND consumed_at IS NULL AND expires_at > NOW()
-- so a captured signed challenge can be exchanged for tokens exactly once.
--
-- refresh_tokens.token_hash is sha256(refresh token) in hex; the raw token is
-- never stored. Each refresh revokes the presented token and issues a new
-- one, and the role is re-derived from indexer state at that point.
--
-- Safe to re-run: CREATE TABLE IF NOT EXISTS is idempotent.
--
-- Cleanup (both tables):
--   DELETE FROM sep10_challenges WHERE expires_at < NOW() - INTERVAL '1 day';
--   DELETE FROM refresh_tokens   WHERE expires_at < NOW() - INTERVAL '1 day';

CREATE TABLE IF NOT EXISTS sep10_challenges (
    hash         VARCHAR(64)  PRIMARY KEY,
    account      VARCHAR(56)  NOT NULL,
    expires_at   TIMESTAMPTZ  NOT NULL,
    consumed_at  TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_sep10_challenges_expires_at
    ON sep10_challenges (expires_at);

CREATE TABLE IF NOT EXISTS refresh_tokens (
    token_hash   VARCHAR(64)  PRIMARY KEY,
    account      VARCHAR(56)  NOT NULL,
    expires_at   TIMESTAMPTZ  NOT NULL,
    revoked_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at
    ON refresh_tokens (expires_at);

COMMENT ON TABLE sep10_challenges IS
    'Issued SEP-10 challenges. consumed_at is set atomically on first successful '
    'verification; a second submission of the same challenge is rejected.';

COMMENT ON TABLE refresh_tokens IS
    'Rotating refresh tokens (sha256 hashes) for SEP-10 sessions. Refreshing '
    're-derives the account role from indexer state.';

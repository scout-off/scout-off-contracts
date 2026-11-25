-- Refresh-token families and revocable access-token sessions.

CREATE TABLE IF NOT EXISTS sessions (
    id              TEXT        PRIMARY KEY,
    account         TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at      TIMESTAMPTZ,
    revoked_reason  TEXT
);

CREATE INDEX IF NOT EXISTS sessions_account_revoked_at_idx
    ON sessions (account, revoked_at);

CREATE TABLE IF NOT EXISTS refresh_tokens (
    id              TEXT        PRIMARY KEY,
    session_id      TEXT        NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    role            TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at      TIMESTAMPTZ,
    rotated_at      TIMESTAMPTZ,
    replaced_by     TEXT
);

CREATE INDEX IF NOT EXISTS refresh_tokens_session_revoked_at_idx
    ON refresh_tokens (session_id, revoked_at);

CREATE TABLE IF NOT EXISTS revoked_tokens (
    jti             TEXT        PRIMARY KEY,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Evidence data keys are encrypted with an application KMS/master key before
-- being stored. The plaintext key is never persisted in PostgreSQL.
CREATE TABLE IF NOT EXISTS evidence_data_keys (
    player_id           BIGINT      NOT NULL,
    milestone_index     INTEGER     NOT NULL CHECK (milestone_index > 0),
    encrypted_data_key  BYTEA       NOT NULL,
    nonce               BYTEA       NOT NULL CHECK (octet_length(nonce) = 12),
    auth_tag            BYTEA       NOT NULL CHECK (octet_length(auth_tag) = 16),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (player_id, milestone_index),
    FOREIGN KEY (player_id, milestone_index)
        REFERENCES milestones (player_id, milestone_index)
);

COMMENT ON TABLE evidence_data_keys IS
    'AES-256-GCM encrypted per-milestone data keys; ciphertext only, never plaintext.';

-- This table is append-only. A key response is not sent until its audit row
-- commits successfully.
CREATE TABLE IF NOT EXISTS evidence_key_releases (
    id                  BIGSERIAL   PRIMARY KEY,
    player_id           BIGINT      NOT NULL,
    milestone_index     INTEGER     NOT NULL,
    requester_wallet    VARCHAR(56) NOT NULL,
    requester_role      VARCHAR(16) NOT NULL
        CHECK (requester_role IN ('scout', 'player')),
    -- Current EvidenceAccessGrant records do not expire. This remains NULL
    -- unless a future contract version adds an explicit grant expiry.
    grant_expires_at    BIGINT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_evidence_key_releases_player_created
    ON evidence_key_releases (player_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_evidence_key_releases_requester_created
    ON evidence_key_releases (requester_wallet, created_at DESC);

CREATE TABLE IF NOT EXISTS revoked_tokens (
    jti         VARCHAR(256) PRIMARY KEY,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

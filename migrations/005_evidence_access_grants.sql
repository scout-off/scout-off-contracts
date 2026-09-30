-- migrations/005_evidence_access_grants.sql
-- Off-chain mirror of scout_access.EvidenceAccessGrant — see
-- docs/EVIDENCE_PRIVACY.md and docs/INDEXER.md.
--
-- A row here is an append-only fact ("this scout was granted access to this
-- player's confidential evidence at this time, at this tier"), not a live
-- entitlement. `revoked` / `revoked_at` are only ever set by
-- scout_access.admin_revoke_evidence_access — never by subscription
-- downgrade or expiry reconciliation, matching the on-chain design. See
-- docs/EVIDENCE_PRIVACY.md — "Grant lifecycle" for why revocation here only
-- gates *future* key-wrap requests and cannot claw back an already-delivered
-- wrapped key.
--
-- Safe to re-run: CREATE TABLE IF NOT EXISTS is idempotent.

CREATE TABLE IF NOT EXISTS evidence_access_grants (
    id              SERIAL       PRIMARY KEY,
    player_id       BIGINT       NOT NULL REFERENCES players (player_id),
    scout           VARCHAR(56)  NOT NULL,
    -- Ledger timestamp (Unix seconds) the grant was issued at, from the
    -- evidence_access_granted event / EvidenceAccessGrant.granted_at.
    granted_at      BIGINT       NOT NULL,
    -- The scout's subscription tier at the moment of grant issuance.
    -- Recorded for audit purposes only; not re-checked afterward.
    tier_at_grant   VARCHAR(16)  NOT NULL CHECK (tier_at_grant IN ('Basic', 'Pro', 'Elite')),
    revoked         BOOLEAN      NOT NULL DEFAULT FALSE,
    -- Ledger timestamp (Unix seconds) of admin_revoke_evidence_access, if any.
    revoked_at      BIGINT,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (player_id, scout)
);

CREATE INDEX IF NOT EXISTS idx_evidence_access_grants_player
    ON evidence_access_grants (player_id);

CREATE INDEX IF NOT EXISTS idx_evidence_access_grants_scout
    ON evidence_access_grants (scout);

COMMENT ON TABLE evidence_access_grants IS
    'Off-chain mirror of scout_access.EvidenceAccessGrant, populated from the '
    'evidence_access_granted / evidence_access_revoked events. The '
    'frontend/backend key-wrapping service reads this table (or the '
    'contract directly) before honoring a wrapped-decryption-key request; '
    'see docs/EVIDENCE_PRIVACY.md.';

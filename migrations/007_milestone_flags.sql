-- Migration 007: milestone_flags — validator revocation cascade re-review (issue #1039)
--
-- Adds two new tables:
--   milestone_flags       — tracks which milestones are currently flagged as
--                           pending re-review due to a for-cause validator
--                           revocation cascade.
--   revocation_records    — mirrors the on-chain RevocationRecord stored under
--                           DataKey::RevocationRecord(wallet) so the indexer
--                           can surface revocation severity and reason without
--                           a per-validator contract query.
--
-- Both tables are idempotent (IF NOT EXISTS) and safe to re-run.

-- ── milestone_flags ──────────────────────────────────────────────────────────
--
-- One row per currently-flagged (player_id, milestone_index) pair.
-- Row is inserted when the indexer observes a `milestone_flagged` event
-- (emitted by run_cascade_sweep for each flagged milestone).
-- Row is deleted when the indexer observes a `milestone_flag_cleared` event
-- (emitted by rereview_milestone on successful clearance).
--
-- The `flagged_by_validator` column holds the wallet address of the revoked
-- validator whose cascade sweep flagged this milestone.

CREATE TABLE IF NOT EXISTS milestone_flags (
    player_id           BIGINT      NOT NULL,
    milestone_index     INTEGER     NOT NULL,
    flagged_by_validator TEXT       NOT NULL,
    flagged_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (player_id, milestone_index)
);

CREATE INDEX IF NOT EXISTS milestone_flags_validator_idx
    ON milestone_flags (flagged_by_validator);

COMMENT ON TABLE milestone_flags IS
    'Milestones flagged as pending re-review due to a for-cause validator '
    'revocation cascade (issue #1039). Populated from milestone_flagged events; '
    'rows deleted on milestone_flag_cleared events.';

-- ── revocation_records ───────────────────────────────────────────────────────
--
-- One row per revoked validator wallet (upserted so re-revocations overwrite).
-- severity: 'Routine' | 'ForCause'
-- cascade_complete: true once the entire ValidatorMilestones history has been
--   flagged (set when the indexer observes revocation_cascade_complete).

CREATE TABLE IF NOT EXISTS revocation_records (
    validator_wallet    TEXT        NOT NULL PRIMARY KEY,
    severity            TEXT        NOT NULL CHECK (severity IN ('Routine', 'ForCause')),
    reason              TEXT        NOT NULL DEFAULT '',
    revoked_at          TIMESTAMPTZ NOT NULL,
    admin_wallet        TEXT        NOT NULL,
    cascade_complete    BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE revocation_records IS
    'Off-chain mirror of the on-chain RevocationRecord stored under '
    'DataKey::RevocationRecord(wallet). Populated from validator_revoked / '
    'validator_revoked_for_cause events and updated when '
    'revocation_cascade_complete is observed (issue #1039).';

-- ── reconcile-indexer support ────────────────────────────────────────────────
--
-- The reconcile-indexer script diffs on-chain state against the DB.
-- Add an index so the script can efficiently query all flagged milestones.

CREATE INDEX IF NOT EXISTS milestone_flags_player_idx
    ON milestone_flags (player_id);

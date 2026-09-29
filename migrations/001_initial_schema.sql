-- ScoutChain — initial PostgreSQL schema
-- Run by the backend on first startup or via a migration tool (e.g. node-pg-migrate)
-- Note: CREATE TABLE IF NOT EXISTS does not retroactively add constraints to existing tables.
-- Existing deployed databases require a companion ALTER TABLE ... ADD CONSTRAINT migration.

-- -----------------------------------------------------------------------
-- Players
-- Known gap: player deactivation status is not tracked here.
-- registration.deactivate_player / reactivate_player have no corresponding
-- column in this table.  See docs/INDEXER.md — "Known gaps" for details.
-- -----------------------------------------------------------------------
-- Note: the deactivated column was added in #837 to track
-- registration.deactivate_player / reactivate_player events.
-- This resolves the "Known gap" previously documented in docs/INDEXER.md.
-- Reconciliation for this column is tracked in #1060.
CREATE TABLE IF NOT EXISTS players (
    player_id       BIGINT PRIMARY KEY,
    wallet          VARCHAR(56)  NOT NULL UNIQUE,   -- Stellar G-address
    age             INTEGER      NOT NULL,
    position        VARCHAR(64)  NOT NULL,
    region          VARCHAR(128) NOT NULL,
    nationality     VARCHAR(128) NOT NULL,
    ipfs_hashes     TEXT[]       NOT NULL DEFAULT '{}',
    level           SMALLINT     NOT NULL DEFAULT 0, -- 0-3
    deactivated     BOOLEAN      NOT NULL DEFAULT FALSE,
    registered_at   BIGINT       NOT NULL,           -- Unix timestamp
    updated_at      BIGINT       NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_players_region   ON players (region);
CREATE INDEX IF NOT EXISTS idx_players_position ON players (position);
CREATE INDEX IF NOT EXISTS idx_players_level    ON players (level);
CREATE INDEX IF NOT EXISTS idx_players_wallet   ON players (wallet);

-- -----------------------------------------------------------------------
-- Player level history (progress.progress_updated / progress.player_level_reset)
-- Distinguishes normal progression (advance_level) from admin corrections
-- (reset_player_level), matching the two contract code paths that change level.
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS player_level_history (
    id              SERIAL       PRIMARY KEY,
    player_id       BIGINT       NOT NULL REFERENCES players (player_id),
    old_level       SMALLINT     NOT NULL,
    new_level       SMALLINT     NOT NULL,
    source          VARCHAR(16)  NOT NULL CHECK (source IN ('advance', 'reset')),
    updated_by      VARCHAR(56),           -- caller Address for progress_updated; NULL for admin reset
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_player_level_history_player ON player_level_history (player_id);

-- -----------------------------------------------------------------------
-- Scouts
-- Known gap: the `verified` column below is not yet populated by the
-- indexer — registration.get_scout(...).verified exists on-chain but
-- the event stream currently has no field to drive this column.
-- See docs/INDEXER.md — "Known gaps" for details.
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS scouts (
    scout_id        BIGINT       PRIMARY KEY,
    wallet          VARCHAR(56)  NOT NULL UNIQUE,
    region          VARCHAR(128) NOT NULL,
    verified        BOOLEAN      NOT NULL DEFAULT FALSE, -- mirrors registration.get_scout(...).verified
    registered_at   BIGINT       NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Companion migration for already-deployed databases
-- (CREATE TABLE IF NOT EXISTS does not add columns to an existing table):
--
--   ALTER TABLE scouts
--     ADD COLUMN IF NOT EXISTS verified BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS idx_scouts_wallet ON scouts (wallet);

-- -----------------------------------------------------------------------
-- Validators
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS validators (
    wallet          VARCHAR(56)  PRIMARY KEY,
    credentials     TEXT         NOT NULL,
    active          BOOLEAN      NOT NULL DEFAULT TRUE,
    registered_at   BIGINT       NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- -----------------------------------------------------------------------
-- Validator history (validator_restored / validator_transferred)
-- The validators table only reflects current state; this table is the
-- audit trail of restore and wallet-transfer events over time.
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS validator_history (
    id              SERIAL       PRIMARY KEY,
    event_type      VARCHAR(16)  NOT NULL CHECK (event_type IN ('restored', 'transferred')),
    old_wallet      VARCHAR(56),           -- set for 'transferred'
    new_wallet      VARCHAR(56)  NOT NULL, -- restored wallet, or the transfer's destination wallet
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_validator_history_new_wallet ON validator_history (new_wallet);
CREATE INDEX IF NOT EXISTS idx_validator_history_old_wallet ON validator_history (old_wallet);

-- -----------------------------------------------------------------------
-- Milestones
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS milestones (
    id              SERIAL       PRIMARY KEY,
    player_id       BIGINT       NOT NULL REFERENCES players (player_id),
    milestone_index INTEGER      NOT NULL CHECK (milestone_index > 0),           -- index within the contract
    validator       VARCHAR(56)  NOT NULL,
    description     TEXT         NOT NULL,
    evidence_hash   VARCHAR(256) NOT NULL,           -- IPFS CID
    approved_at     BIGINT       NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (player_id, milestone_index)
);

CREATE INDEX IF NOT EXISTS idx_milestones_player ON milestones (player_id);

-- -----------------------------------------------------------------------
-- Milestone disputes (verification.milestone_disputed / dispute_resolved)
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS milestone_disputes (
    id              SERIAL       PRIMARY KEY,
    player_id       BIGINT       NOT NULL REFERENCES players (player_id),
    milestone_index INTEGER      NOT NULL CHECK (milestone_index > 0),
    reason          TEXT         NOT NULL,
    disputed_at     BIGINT       NOT NULL,           -- Unix timestamp
    resolved        BOOLEAN      NOT NULL DEFAULT FALSE,
    upheld          BOOLEAN      NOT NULL DEFAULT FALSE,
    resolved_at     TIMESTAMPTZ,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (player_id, milestone_index)
);

CREATE INDEX IF NOT EXISTS idx_milestone_disputes_player ON milestone_disputes (player_id);

-- -----------------------------------------------------------------------
-- Scout subscriptions
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS scout_subscriptions (
    scout           VARCHAR(56)  PRIMARY KEY,
    tier            VARCHAR(16)  NOT NULL CHECK (tier IN ('Basic', 'Pro', 'Elite')), -- Basic | Pro | Elite
    subscribed_at   BIGINT       NOT NULL,
    expires_at      BIGINT       NOT NULL,
    updated_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- -----------------------------------------------------------------------
-- Fee config history (scout_access.fee_config_updated) — audit trail of
-- FeeConfig changes; scout_subscriptions only reflects live subscriptions.
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS fee_config_history (
    id                          SERIAL      PRIMARY KEY,
    old_contact_fee_stroops     BIGINT      NOT NULL,
    old_basic_sub_stroops       BIGINT      NOT NULL,
    old_pro_sub_stroops         BIGINT      NOT NULL,
    old_elite_sub_stroops       BIGINT      NOT NULL,
    old_sub_duration_secs       BIGINT      NOT NULL,
    new_contact_fee_stroops     BIGINT      NOT NULL,
    new_basic_sub_stroops       BIGINT      NOT NULL,
    new_pro_sub_stroops         BIGINT      NOT NULL,
    new_elite_sub_stroops       BIGINT      NOT NULL,
    new_sub_duration_secs       BIGINT      NOT NULL,
    created_db_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- -----------------------------------------------------------------------
-- Contact records
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS contact_records (
    id              SERIAL       PRIMARY KEY,
    scout           VARCHAR(56)  NOT NULL,
    player_id       BIGINT       NOT NULL REFERENCES players (player_id),
    contacted_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (scout, player_id)
);

CREATE INDEX IF NOT EXISTS idx_contacts_scout  ON contact_records (scout);
CREATE INDEX IF NOT EXISTS idx_contacts_player ON contact_records (player_id);

-- -----------------------------------------------------------------------
-- Trial offers
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS trial_offers (
    id              SERIAL       PRIMARY KEY,
    player_id       BIGINT       NOT NULL REFERENCES players (player_id),
    trial_index     INTEGER      NOT NULL CHECK (trial_index > 0),
    scout           VARCHAR(56)  NOT NULL,
    details_hash    VARCHAR(256) NOT NULL,           -- IPFS CID
    logged_at       BIGINT       NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (player_id, trial_index)
);

CREATE INDEX IF NOT EXISTS idx_trials_player ON trial_offers (player_id);
CREATE INDEX IF NOT EXISTS idx_trials_scout  ON trial_offers (scout);

-- -----------------------------------------------------------------------
-- Fee withdrawals (audit log)
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS fee_withdrawals (
    id              SERIAL       PRIMARY KEY,
    recipient       VARCHAR(56)  NOT NULL,
    amount_stroops  BIGINT       NOT NULL,
    withdrawn_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- -----------------------------------------------------------------------
-- Admin transfers (registration.admin_transferred / verification.admin_transferred /
-- progress.admin_transferred / scout_access.admin_transferred)
-- All four contracts implement propose_admin/accept_admin and emit the
-- admin_transferred event, so contract_name must cover all four.
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS admin_transfers (
    id              SERIAL       PRIMARY KEY,
    contract_name   VARCHAR(32)  NOT NULL CHECK (contract_name IN ('registration', 'verification', 'progress', 'scout_access')),
    old_admin       VARCHAR(56)  NOT NULL,
    new_admin       VARCHAR(56)  NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Companion migration for already-deployed databases
-- (CREATE TABLE IF NOT EXISTS does not modify constraints on existing tables):
--
--   ALTER TABLE admin_transfers
--     DROP CONSTRAINT IF EXISTS admin_transfers_contract_name_check;
--   ALTER TABLE admin_transfers
--     ADD CONSTRAINT admin_transfers_contract_name_check
--       CHECK (contract_name IN ('registration', 'verification', 'progress', 'scout_access'));

CREATE INDEX IF NOT EXISTS idx_admin_transfers_contract ON admin_transfers (contract_name);

-- -----------------------------------------------------------------------
-- Event cursor (indexer checkpoint)
-- Single-row table that records the last Horizon ledger sequence the
-- backend indexer has fully processed.  The indexer SELECTs this row
-- on startup and uses last_ledger as the starting point for its next
-- Horizon events query.
--
-- Cursor reset procedure (event replay from genesis):
--   To replay all on-chain events from the beginning (e.g. after a full
--   database wipe or to reindex a new environment), reset the cursor to 0:
--
--     UPDATE indexer_cursor SET last_ledger = 0, updated_at = NOW()
--     WHERE id = 1;
--
--   The indexer will then restart from ledger 0 on its next poll cycle.
--   Truncate any derived tables (players, milestones, etc.) before
--   replaying to avoid duplicate-key errors from re-processing events.
--   See docs/DEPLOYMENT.md — Resetting the Indexer Cursor for the full
--   step-by-step procedure.
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS indexer_cursor (
    id              INTEGER      PRIMARY KEY DEFAULT 1,  -- single row
    last_ledger     BIGINT       NOT NULL DEFAULT 0,
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    CHECK (id = 1)
);

-- Seed the single cursor row so the indexer can SELECT it on first startup
-- without encountering an empty result.  ON CONFLICT DO NOTHING makes this
-- safe to re-run against an already-migrated database.
INSERT INTO indexer_cursor (id, last_ledger, updated_at)
VALUES (1, 0, NOW())
ON CONFLICT (id) DO NOTHING;

-- -----------------------------------------------------------------------
-- Retroactive migrations for already-deployed databases
-- -----------------------------------------------------------------------
-- These ALTER TABLE statements are idempotent and can be run against an
-- existing database to add columns introduced after initial deployment.

ALTER TABLE players ADD COLUMN IF NOT EXISTS deactivated BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE scouts ADD COLUMN IF NOT EXISTS verified BOOLEAN NOT NULL DEFAULT FALSE;


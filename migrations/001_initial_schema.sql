-- ScoutChain — initial PostgreSQL schema
-- Run by the backend on first startup or via a migration tool (e.g. node-pg-migrate)

-- -----------------------------------------------------------------------
-- Players
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS players (
    player_id       BIGINT PRIMARY KEY,
    wallet          VARCHAR(56)  NOT NULL UNIQUE,   -- Stellar G-address
    age             INTEGER      NOT NULL,
    position        VARCHAR(64)  NOT NULL,
    region          VARCHAR(128) NOT NULL,
    nationality     VARCHAR(128) NOT NULL,
    ipfs_hashes     TEXT[]       NOT NULL DEFAULT '{}',
    level           SMALLINT     NOT NULL DEFAULT 0, -- 0-3
    registered_at   BIGINT       NOT NULL,           -- Unix timestamp
    updated_at      BIGINT       NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_players_region   ON players (region);
CREATE INDEX IF NOT EXISTS idx_players_position ON players (position);
CREATE INDEX IF NOT EXISTS idx_players_level    ON players (level);
CREATE INDEX IF NOT EXISTS idx_players_wallet   ON players (wallet);

-- -----------------------------------------------------------------------
-- Scouts
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS scouts (
    scout_id        BIGINT PRIMARY KEY,
    wallet          VARCHAR(56)  NOT NULL UNIQUE,
    region          VARCHAR(128) NOT NULL,
    registered_at   BIGINT       NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

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
-- Milestones
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS milestones (
    id              SERIAL       PRIMARY KEY,
    player_id       BIGINT       NOT NULL REFERENCES players (player_id),
    milestone_index INTEGER      NOT NULL,           -- index within the contract
    validator       VARCHAR(56)  NOT NULL,
    description     TEXT         NOT NULL,
    evidence_hash   VARCHAR(256) NOT NULL,           -- IPFS CID
    approved_at     BIGINT       NOT NULL,
    created_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (player_id, milestone_index)
);

CREATE INDEX IF NOT EXISTS idx_milestones_player ON milestones (player_id);

-- -----------------------------------------------------------------------
-- Scout subscriptions
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS scout_subscriptions (
    scout           VARCHAR(56)  PRIMARY KEY,
    tier            VARCHAR(16)  NOT NULL,           -- Basic | Pro | Elite
    subscribed_at   BIGINT       NOT NULL,
    expires_at      BIGINT       NOT NULL,
    updated_db_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
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
    trial_index     INTEGER      NOT NULL,
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
-- Event cursor (indexer checkpoint)
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS indexer_cursor (
    id              INTEGER      PRIMARY KEY DEFAULT 1,  -- single row
    last_ledger     BIGINT       NOT NULL DEFAULT 0,
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    CHECK (id = 1)
);

-- Seed the single cursor row so indexers can SELECT it on first startup.
INSERT INTO indexer_cursor (id, last_ledger) VALUES (1, 0)
ON CONFLICT (id) DO NOTHING;

-- Idempotently advance or reset the event cursor.
-- To replay events from genesis after rebuilding derived tables, run:
--   SELECT upsert_indexer_cursor(0);
CREATE OR REPLACE FUNCTION upsert_indexer_cursor(p_last_ledger BIGINT)
RETURNS VOID AS $$
    INSERT INTO indexer_cursor (id, last_ledger, updated_at)
    VALUES (1, p_last_ledger, NOW())
    ON CONFLICT (id) DO UPDATE
    SET last_ledger = EXCLUDED.last_ledger,
        updated_at = EXCLUDED.updated_at;
$$ LANGUAGE SQL;

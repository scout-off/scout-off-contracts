-- -----------------------------------------------------------------------
-- Admins
-- Stores addresses that are authorised to perform admin operations
-- (e.g. token revocation, contract pausing).  Populated from on-chain
-- admin registry events or the contract's current admin address.
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS admins (
    address     VARCHAR(56) PRIMARY KEY,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

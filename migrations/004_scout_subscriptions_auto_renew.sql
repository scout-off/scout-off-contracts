-- migrations/004_scout_subscriptions_auto_renew.sql
-- Adds the auto_renew column to scout_subscriptions so the off-chain index
-- can track the per-scout auto-renewal opt-in, mirroring
-- scout_access.get_auto_renew(scout).
--
-- This closes a reconciliation gap (see docs/INDEXER.md, issue #1015):
-- reconcile-indexer.js could not detect drift on the auto-renew flag because
-- the DB had nowhere to store it, and no indexer path wrote it.
--
-- Safe to re-run: ALTER TABLE ... ADD COLUMN IF NOT EXISTS is idempotent.
--
-- NULL is never a valid state for this column — the contract's get_auto_renew
-- always returns a concrete boolean, and a row written before the indexer
-- populates this field defaults to FALSE (the contract's own default for a
-- scout who has never opted in), which reconcile-indexer.js's auto_renew
-- check will then compare against and correct on the next reconciliation run.

ALTER TABLE scout_subscriptions ADD COLUMN IF NOT EXISTS auto_renew BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN scout_subscriptions.auto_renew IS
    'Mirrors scout_access.get_auto_renew(scout). Populated by the indexer on '
    'subscribe/renew events; reconciled by reconcile-indexer.js against the '
    'on-chain getter.';

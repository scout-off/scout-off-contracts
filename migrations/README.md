# Database Migrations

PostgreSQL migration files for the ScoutChain backend event indexer.

## Apply Order

**Run every file in numeric order.** Skipping any migration leaves tables,
columns, or indexes missing and will cause silent indexer errors at runtime.

```bash
psql $DATABASE_URL -f migrations/001_initial_schema.sql
psql $DATABASE_URL -f migrations/002_cursor_upsert_helper.sql
psql $DATABASE_URL -f migrations/003_diagnostic_events.sql
psql $DATABASE_URL -f migrations/004_scout_subscriptions_auto_renew.sql
psql $DATABASE_URL -f migrations/005_evidence_access_grants.sql
psql $DATABASE_URL -f migrations/006_dispute_jury.sql
psql $DATABASE_URL -f migrations/007_milestone_flags.sql
psql $DATABASE_URL -f migrations/008_session_management.sql
```

All migration files are idempotent — every `CREATE TABLE`, `CREATE INDEX`, and
`ALTER TABLE … ADD COLUMN` statement uses `IF NOT EXISTS`. It is safe to re-run
any file against an already-migrated database.

The session-management migration adds the auth API's session-family tables;
apply it to the same PostgreSQL database configured by `DATABASE_URL`.

## File Reference

| File | What it creates / modifies |
|------|---------------------------|
| `001_initial_schema.sql` | Fourteen base tables: `players`, `player_level_history`, `scouts`, `validators`, `validator_history`, `milestones`, `milestone_disputes`, `scout_subscriptions`, `fee_config_history`, `contact_records`, `trial_offers`, `fee_withdrawals`, `admin_transfers`, `indexer_cursor` |
| `002_cursor_upsert_helper.sql` | `advance_indexer_cursor(p_ledger BIGINT)` helper function |
| `003_diagnostic_events.sql` | `diagnostic_events` table |
| `004_scout_subscriptions_auto_renew.sql` | `auto_renew` column on `scout_subscriptions` |
| `005_evidence_access_grants.sql` | `evidence_access_grants` table (off-chain mirror of `scout_access.EvidenceAccessGrant`) |
| `006_dispute_jury.sql` | Jury-escalation columns on `milestone_disputes`; `dispute_votes` table |
| `007_milestone_flags.sql` | `milestone_flags` and `revocation_records` tables |
| `008_session_management.sql` | `sessions`, `refresh_tokens`, and `revoked_tokens` tables for refresh-token families and logout |

## Related Documentation

- [docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md) — full deployment guide including the migration step
- [docs/AUTH_SESSIONS.md](../docs/AUTH_SESSIONS.md) — session lifecycle and auth endpoint semantics
- `npm run check:auth-schema` — compare the live auth tables and foreign key with the session model
- [README.md — Database Schema](../README.md#database-schema) — table-level overview
- [docs/INDEXER.md](../docs/INDEXER.md) — reconciliation tool and indexer documentation

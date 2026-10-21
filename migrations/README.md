# ScoutChain — Database Migrations

This directory contains the PostgreSQL schema migrations for the ScoutChain
backend event indexer. Each migration is a numbered SQL file that is applied
in ascending order to bring the database schema up to date.

## Migration ownership

**SQL migrations are the canonical source of truth for the database schema.**

The project uses two schema representations:

| Representation | Location | Authority |
|----------------|----------|-----------|
| SQL migrations | `migrations/*.sql` | **Canonical** — applied by the backend on startup or by a DBA |
| Prisma schema | `prisma/schema.prisma` | **Derived** — must be kept in sync with the SQL migrations |

When the two representations diverge, the SQL migration wins. If you add or
change a table, write the SQL migration first, then update `prisma/schema.prisma`
to match. Never apply a `prisma migrate` command against a production database
without a corresponding SQL migration in this directory.

## Prisma mapping convention

Prisma model names use PascalCase; the corresponding SQL table names use
snake_case. Every Prisma model must declare a `@@map` annotation that names
the SQL table:

```prisma
model RevokedToken {
  jti       String   @id @db.VarChar(128)
  createdAt DateTime @default(now()) @map("created_at") @db.Timestamptz

  @@map("revoked_tokens")
}
```

This ensures that Prisma-generated queries target the correct table name
regardless of the Prisma model name.

## Naming and numbering convention

Files must be named `NNN_description.sql` where `NNN` is a zero-padded
three-digit integer incremented by one for each new migration. Numbers must be
unique and contiguous; `scripts/check-migration-numbering.sh` enforces this in
CI. Never reuse or skip a number, and never rename a migration after it has
been applied to any environment (local, testnet, or mainnet).

> **Renumbering note (October 2026).** Four migrations were merged with
> numbers that were already taken (two `004_*` and four `008_*` files). The
> earliest file at each number kept it; the later ones were moved to the end
> in the order they landed: `004_revoked_tokens` → `009`, `008_deactivated_scout`
> → `010`, `008_admins` → `011`, `008_dispute_rounds` → `012`. All four are
> idempotent, so a database that already applied them under the old names can
> safely run the new files.

## Idempotency requirement

Every migration must be safe to re-run against an already-migrated database.
Use `IF NOT EXISTS` guards for `CREATE TABLE`, `CREATE INDEX`, `CREATE FUNCTION`,
and `CREATE TYPE`, `ADD COLUMN IF NOT EXISTS` for new columns,
`DROP CONSTRAINT IF EXISTS` before re-adding a constraint, and
`ON CONFLICT DO NOTHING` for seed `INSERT` statements.

```sql
CREATE TABLE IF NOT EXISTS my_table (
    id   SERIAL PRIMARY KEY,
    name TEXT   NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_my_table_name ON my_table (name);
```

## Applying migrations

**Run every file in numeric order.** Skipping one leaves tables, columns or
indexes missing and causes silent indexer errors at runtime.

```bash
for f in migrations/*.sql; do
    echo "Applying $f..."
    psql "$DATABASE_URL" -f "$f"
done
```

The backend service applies all migrations automatically on startup in the
same order. See [docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md) for the full
deployment guide.

## Current migrations

| File | What it creates / modifies |
|------|---------------------------|
| `001_initial_schema.sql` | Fourteen base tables: `players`, `player_level_history`, `scouts`, `validators`, `validator_history`, `milestones`, `milestone_disputes`, `scout_subscriptions`, `fee_config_history`, `contact_records`, `trial_offers`, `fee_withdrawals`, `admin_transfers`, `indexer_cursor` |
| `002_cursor_upsert_helper.sql` | `advance_indexer_cursor(p_ledger BIGINT)` helper function |
| `003_diagnostic_events.sql` | `diagnostic_events` table for observability events (level advancement skipped, progress contract not wired, cross-contract call failures) |
| `004_scout_subscriptions_auto_renew.sql` | `auto_renew` column on `scout_subscriptions` |
| `005_evidence_access_grants.sql` | `evidence_access_grants` table (off-chain mirror of `scout_access.EvidenceAccessGrant`) |
| `006_dispute_jury.sql` | Jury-escalation columns on `milestone_disputes`; `dispute_votes` table |
| `007_milestone_flags.sql` | `milestone_flags` and `revocation_records` tables |
| `008_align_player_column_limits.sql` | Aligns `players` column sizes with the registration contract's string limits |
| `009_revoked_tokens.sql` | `revoked_tokens` table for explicit JWT revocation on sign-out and credential rotation |
| `010_deactivated_scout.sql` | `deactivated` column on `scouts` |
| `011_admins.sql` | `admins` table of addresses authorised for admin operations |
| `012_dispute_rounds.sql` | Dispute rounds (#1396): `round` / `resolved_at_unix` on `milestone_disputes` and `round` on `dispute_votes`, with round-aware unique keys |

## Related documentation

- [docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md) — full deployment guide including the migration step
- [README.md — Database Schema](../README.md#database-schema) — table-level overview

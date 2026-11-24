'use strict';

/**
 * Structural + behavioural tests for scripts/reconcile-indexer.js.
 *
 * These tests guard against the class of bug where helper functions are
 * accidentally nested inside another function (syntactically valid, but a
 * ReferenceError at runtime). They also exercise `main` end-to-end with a
 * mocked `invoke` and `pg` so every table in ALL_TABLES is reconciled.
 */

const path = require('path');

const SCRIPT_PATH = path.resolve(__dirname, '..', 'reconcile-indexer.js');

// Minimal in-memory pg mock. Each query returns an empty result set unless a
// test seeds rows via `__setRows`.
function createPgMock() {
  const rowsByTable = new Map();
  const client = {
    query: jest.fn(async (sql) => {
      const match = /FROM\s+([a-z_]+)/i.exec(String(sql));
      const table = match ? match[1] : null;
      const rows = table && rowsByTable.has(table) ? rowsByTable.get(table) : [];
      return { rows };
    }),
    end: jest.fn(async () => {}),
  };
  return {
    client,
    __setRows(table, rows) {
      rowsByTable.set(table, rows);
    },
  };
}

function loadScript() {
  jest.resetModules();
  return require(SCRIPT_PATH);
}

describe('scripts/reconcile-indexer.js structure', () => {
  test('reconcileEvidenceAccessGrants and reconcileDisputeVotes are module-scope', () => {
    const mod = loadScript();
    expect(typeof mod.reconcileEvidenceAccessGrants).toBe('function');
    expect(typeof mod.reconcileDisputeVotes).toBe('function');
    expect(typeof mod.reconcileContactRecords).toBe('function');
    expect(typeof mod.main).toBe('function');
  });
});

describe('scripts/reconcile-indexer.js main()', () => {
  test('runs every table without throwing', async () => {
    const mod = loadScript();
    const pg = createPgMock();
    const invoke = jest.fn(async () => ({ ok: true, value: [] }));

    await expect(
      mod.main({ pg: pg.client, invoke, tables: mod.ALL_TABLES })
    ).resolves.not.toThrow();

    expect(pg.client.end).toHaveBeenCalled();
  });

  test('reconciles dispute_votes and evidence_access_grants without ReferenceError', async () => {
    const mod = loadScript();
    const pg = createPgMock();
    const invoke = jest.fn(async () => ({ ok: true, value: [] }));

    await expect(
      mod.main({
        pg: pg.client,
        invoke,
        tables: ['dispute_votes', 'evidence_access_grants'],
      })
    ).resolves.not.toThrow();
  });

  test('detects a seeded discrepancy in dispute_votes', async () => {
    const mod = loadScript();
    const pg = createPgMock();
    // DB has a row that the chain does not report -> should be flagged.
    pg.__setRows('dispute_votes', [{ dispute_id: '1', voter: 'GABC' }]);
    const invoke = jest.fn(async () => ({ ok: true, value: [] }));

    const report = mod.createReport ? mod.createReport() : { entries: [], add(e) { this.entries.push(e); } };

    await mod.main({ pg: pg.client, invoke, tables: ['dispute_votes'], report });

    const entries = report.entries || [];
    expect(entries.length).toBeGreaterThan(0);
  });
});

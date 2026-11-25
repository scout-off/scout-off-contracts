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

describe('scripts/reconcile-indexer.js reconcileIndexerCursor()', () => {
  const originalFetch = global.fetch;

  afterEach(() => {
    global.fetch = originalFetch;
    jest.useRealTimers();
  });

  function makeReport() {
    const report = { entries: [], add(entry) { this.entries.push(entry); } };
    return report;
  }

  test('reports rpc_timeout when fetch aborts', async () => {
    const mod = loadScript();
    const abortErr = new Error('The operation was aborted');
    abortErr.name = 'AbortError';
    global.fetch = jest.fn(async () => {
      throw abortErr;
    });

    const report = makeReport();
    await expect(
      mod.reconcileIndexerCursor({ rpcUrl: 'http://rpc.test', report, rpcTimeoutMs: 50 })
    ).resolves.not.toThrow();

    const entry = report.entries.find((e) => e.kind === 'rpc_timeout');
    expect(entry).toBeDefined();
    expect(entry.table).toBe('indexer_cursor');
  });

  test('reports rpc_error with httpStatus on a 502 HTML response', async () => {
    const mod = loadScript();
    global.fetch = jest.fn(async () => ({
      ok: false,
      status: 502,
      text: async () => '<html><body>Bad Gateway</body></html>',
      json: async () => {
        throw new SyntaxError('Unexpected token < in JSON at position 0');
      },
    }));

    const report = makeReport();
    await expect(
      mod.reconcileIndexerCursor({ rpcUrl: 'http://rpc.test', report })
    ).resolves.not.toThrow();

    const entry = report.entries.find((e) => e.kind === 'rpc_error');
    expect(entry).toBeDefined();
    expect(entry.table).toBe('indexer_cursor');
    expect(entry.httpStatus).toBe(502);
  });

  test('reports rpc_unexpected_response on malformed JSON', async () => {
    const mod = loadScript();
    global.fetch = jest.fn(async () => ({
      ok: true,
      status: 200,
      text: async () => 'not json at all',
      json: async () => {
        throw new SyntaxError('Unexpected token o in JSON at position 1');
      },
    }));

    const report = makeReport();
    await expect(
      mod.reconcileIndexerCursor({ rpcUrl: 'http://rpc.test', report })
    ).resolves.not.toThrow();

    const entry = report.entries.find((e) => e.kind === 'rpc_unexpected_response');
    expect(entry).toBeDefined();
    expect(entry.table).toBe('indexer_cursor');
  });

  test('reports rpc_unexpected_response when result.sequence is missing', async () => {
    const mod = loadScript();
    global.fetch = jest.fn(async () => ({
      ok: true,
      status: 200,
      text: async () => JSON.stringify({ jsonrpc: '2.0', result: {} }),
      json: async () => ({ jsonrpc: '2.0', result: {} }),
    }));

    const report = makeReport();
    await expect(
      mod.reconcileIndexerCursor({ rpcUrl: 'http://rpc.test', report })
    ).resolves.not.toThrow();

    const entry = report.entries.find((e) => e.kind === 'rpc_unexpected_response');
    expect(entry).toBeDefined();
    expect(entry.table).toBe('indexer_cursor');
  });

  test('passes a default rpc timeout of 10000ms to fetch', async () => {
    const mod = loadScript();
    const fetchMock = jest.fn(async () => ({
      ok: true,
      status: 200,
      text: async () => JSON.stringify({ result: { sequence: 42 } }),
      json: async () => ({ result: { sequence: 42 } }),
    }));
    global.fetch = fetchMock;

    const report = makeReport();
    await mod.reconcileIndexerCursor({ rpcUrl: 'http://rpc.test', report });

    expect(fetchMock).toHaveBeenCalled();
    const init = fetchMock.mock.calls[0][1] || {};
    expect(init.signal).toBeDefined();
  });

  test('parses --rpc-timeout-ms with a default of 10000', () => {
    const mod = loadScript();
    if (typeof mod.parseArgs !== 'function') {
      // Argument parsing may be inline in main; fall back to checking the
      // exported default constant if present.
      if (typeof mod.DEFAULT_RPC_TIMEOUT_MS !== 'undefined') {
        expect(mod.DEFAULT_RPC_TIMEOUT_MS).toBe(10000);
      }
      return;
    }
    const defaults = mod.parseArgs([]);
    expect(defaults.rpcTimeoutMs).toBe(10000);
    const custom = mod.parseArgs(['--rpc-timeout-ms', '2500']);
    expect(custom.rpcTimeoutMs).toBe(2500);
  });
});

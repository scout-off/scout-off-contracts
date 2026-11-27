'use strict';

/**
 * Tests for scripts/audit-event-history.js
 *
 * Covers:
 * - fetchEvents pagination (first page, continuation, last page)
 * - fetchEvents with topic filtering
 * - fetchEvents with startLedger (first request only)
 * - fetchEvents error handling (network errors, RPC errors, empty results)
 * - CLI argument parsing
 * - getRpcInfo parsing
 */

const path = require('path');

const SCRIPT_PATH = path.resolve(__dirname, '..', 'audit-event-history.js');
const EVENTS_PAGE_SIZE = 200;

// Helper to create a mock fetch that returns events
function createMockFetch(eventsByLedger) {
  // Build a sorted array of events
  const allEvents = [...eventsByLedger].sort((a, b) => a.ledger - b.ledger);

  return jest.fn(async (url, { method, body }) => {
    const payload = JSON.parse(body);
    const params = payload.params || {};

    // Check for RPC error case
    if (params.failWithError) {
      return {
        ok: false,
        status: 500,
        json: async () => ({
          jsonrpc: '2.0',
          id: payload.id,
          error: {
            code: -32603,
            message: 'Internal error',
            data: 'Mock RPC internal error',
          },
        }),
      };
    }

    // Parse request params
    const filters = params.filters || [];
    const contractIds = filters[0]?.contractIds || [];
    const topicsFilter = filters[0]?.topics;
    const pagination = params.pagination || {};
    const limit = pagination.limit || EVENTS_PAGE_SIZE;
    const cursor = pagination.cursor;
    const startLedger = params.startLedger;

    // Filter events by contract ID
    let filtered = allEvents.filter((ev) => {
      // Check contract ID in topics
      const eventTopic = ev.topics ? ev.topics[0] : '';
      const isMatch = contractIds.some((cid) => eventTopic.startsWith(cid));
      return isMatch;
    });

    // Apply topic filtering if provided
    if (topicsFilter && topicsFilter.length > 0) {
      filtered = filtered.filter((ev) => {
        if (!ev.topics) return false;
        return topicsFilter.some((ft) =>
          ev.topics.some((et) => et === ft || et.startsWith(ft))
        );
      });
    }

    // Determine start index for pagination
    let startIndex = 0;
    if (cursor) {
      // Decode cursor (simple format: ledger:count)
      const parts = cursor.split(':');
      const startLedgerVal = Number(parts[0]);
      const eventCount = Number(parts[1]);
      startIndex = allEvents.findIndex((e) => e.ledger === startLedgerVal);
      if (startIndex !== -1) {
        // Adjust based on how many events we've already returned
        const offset = startIndex + eventCount;
        if (offset < allEvents.length) {
          startIndex = offset;
        }
      }
    } else if (startLedger !== undefined && startLedger !== null) {
      startIndex = allEvents.findIndex((e) => e.ledger === startLedger);
      if (startIndex === -1) {
        startIndex = 0;
      }
    }

    const paginated = filtered.slice(startIndex, startIndex + limit);
    const hasMore = startIndex + limit < filtered.length;

    return {
      ok: true,
      status: 200,
      json: async () => ({
        jsonrpc: '2.0',
        id: payload.id,
        result: {
          events: paginated.map((ev) => ({
            contract_id: ev.contractId,
            ledger: ev.ledger,
            ledger_close_time: new Date(ev.ledger * 1000).toISOString(),
            type: ev.type,
            topics: ev.topics || [],
            data: ev.data ? { json: JSON.stringify(ev.data) } : {},
          })),
          pagination: {
            cursor: hasMore ? `${paginated[paginated.length - 1].ledger}:${paginated.length}` : null,
          },
        },
      }),
    };
  });
}

// Helper to create a mock fetch for getRpcInfo
function createRpcInfoMock(rpcInfo) {
  return jest.fn(async (url, { body }) => {
    const payload = JSON.parse(body);
    const method = payload.method;

    if (method === 'getHealth') {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          jsonrpc: '2.0',
          id: payload.id,
          result: {
            status: 'healthy',
          },
        }),
      };
    }

    if (method === 'getLatestLedger') {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          jsonrpc: '2.0',
          id: payload.id,
          result: {
            ledger: rpcInfo.latestLedger,
            retention: rpcInfo.retention,
            protocol_version: 21,
          },
        }),
      };
    }

    // Unknown method
    return {
      ok: false,
      status: 400,
      json: async () => ({
        jsonrpc: '2.0',
        id: payload.id,
        error: {
          code: -32601,
          message: 'Method not found',
        },
      }),
    };
  });
}

describe('scripts/audit-event-history.js structure', () => {
  test('fetchEvents is exported', async () => {
    const mod = require(SCRIPT_PATH);
    expect(typeof mod.fetchEvents).toBe('function');
  });

  test('parseArgs is exported', async () => {
    const mod = require(SCRIPT_PATH);
    expect(typeof mod.parseArgs).toBe('function');
  });

  test('requireEnv is exported', async () => {
    const mod = require(SCRIPT_PATH);
    expect(typeof mod.requireEnv).toBe('function');
  });

  test('PlayerHistoryReconstructor is exported', async () => {
    const mod = require(SCRIPT_PATH);
    expect(typeof mod.PlayerHistoryReconstructor).toBe('function');
  });

  test('getRpcInfo is exported', async () => {
    const mod = require(SCRIPT_PATH);
    expect(typeof mod.getRpcInfo).toBe('function');
  });

  test('EVENTS_PAGE_SIZE is exported', async () => {
    const mod = require(SCRIPT_PATH);
    expect(mod.EVENTS_PAGE_SIZE).toBe(200);
  });
});

describe('scripts/audit-event-history.js fetchEvents', () => {
  let originalFetch;

  beforeEach(() => {
    originalFetch = global.fetch;
  });

  afterEach(() => {
    global.fetch = originalFetch;
    jest.restoreAllMocks();
  });

  test('fetches first page when startLedger is provided', async () => {
    const mockEvents = Array.from({ length: 50 }, (_, i) => ({
      contractId: 'C123',
      ledger: 1000 + i,
      type: 'progress_updated',
      topics: ['progress_updated', 'wallet1'],
      data: { player_id: 1, old_level: 'Unverified', new_level: 'VerifiedIdentity' },
    }));

    global.fetch = createMockFetch(mockEvents);

    const mod = require(SCRIPT_PATH);
    const result = await mod.fetchEvents('http://rpc.example.com', 'C123', null, { startLedger: 1000 });

    expect(global.fetch).toHaveBeenCalledTimes(1);
    const request = JSON.parse(global.fetch.mock.calls[0][1].body);
    expect(request.params.startLedger).toBe(1000);
    expect(request.params.filters).toEqual([
      {
        type: 'contract',
        contractIds: ['C123'],
        topics: null,
      },
    ]);
    expect(request.params.pagination.limit).toBe(EVENTS_PAGE_SIZE);

    // Should return all 50 events in one call since they fit in one page
    expect(result.length).toBe(50);
    expect(result[0].ledger).toBe(1000);
    expect(result[result.length - 1].ledger).toBe(1049);
  });

  test('paginates across multiple pages when needed', async () => {
    // Create 450 events (more than 2 pages of 200)
    const mockEvents = Array.from({ length: 450 }, (_, i) => ({
      contractId: 'C123',
      ledger: 2000 + i,
      type: 'progress_updated',
      topics: ['progress_updated', 'wallet1'],
      data: { player_id: 1, old_level: 'Unverified', new_level: 'VerifiedIdentity' },
    }));

    global.fetch = createMockFetch(mockEvents);

    const mod = require(SCRIPT_PATH);
    const result = await mod.fetchEvents('http://rpc.example.com', 'C123', null, { startLedger: 2000 });

    // Should make 3 requests (200 + 200 + 50)
    expect(global.fetch).toHaveBeenCalledTimes(3);

    // Should have all 450 events
    expect(result.length).toBe(450);
    expect(result[0].ledger).toBe(2000);
    expect(result[result.length - 1].ledger).toBe(2449);
  });

  test('applies topic filter when provided', async () => {
    const mockEvents = [
      // progress events
      {
        contractId: 'C123',
        ledger: 3000,
        type: 'progress_updated',
        topics: ['progress_updated', 'wallet1'],
        data: { player_id: 1 },
      },
      // verification event with different topic
      {
        contractId: 'C456',
        ledger: 3001,
        type: 'milestone_approved',
        topics: ['milestone_approved', 'wallet1'],
        data: { player_id: 1 },
      },
    ];

    global.fetch = createMockFetch(mockEvents);

    const mod = require(SCRIPT_PATH);
    // Filter for progress events only
    const result = await mod.fetchEvents('http://rpc.example.com', 'C123', ['progress_updated'], { startLedger: 3000 });

    expect(global.fetch).toHaveBeenCalledTimes(1);
    const request = JSON.parse(global.fetch.mock.calls[0][1].body);

    // Request should include topic filter
    expect(request.params.filters[0].topics).toEqual(['progress_updated']);

    // Should only return progress events
    expect(result.length).toBe(1);
    expect(result[0].type).toBe('progress_updated');
  });

  test('handles empty results gracefully', async () => {
    const mockEvents = [];

    global.fetch = createMockFetch(mockEvents);

    const mod = require(SCRIPT_PATH);
    const result = await mod.fetchEvents('http://rpc.example.com', 'C123', null, { startLedger: 4000 });

    expect(global.fetch).toHaveBeenCalledTimes(1);
    expect(result.length).toBe(0);
  });

  test('throws on network error (non-OK response)', async () => {
    global.fetch = jest.fn(async () => ({
      ok: false,
      status: 503,
      statusText: 'Service Unavailable',
    }));

    const mod = require(SCRIPT_PATH);

    await expect(
      mod.fetchEvents('http://rpc.example.com', 'C123', null, { startLedger: 5000 })
    ).rejects.toThrow('Network error: 503 Service Unavailable');
  });

  test('throws on RPC error response', async () => {
    const mockFetch = createMockFetch([]);
    mockFetch.mockImplementationOnce(() => ({
      ok: true,
      status: 200,
      json: async () => ({
        jsonrpc: '2.0',
        id: 1,
        error: {
          code: -32602,
          message: 'Invalid params',
          data: 'Missing contractIds',
        },
      }),
    }));
    global.fetch = mockFetch;

    const mod = require(SCRIPT_PATH);

    await expect(
      mod.fetchEvents('http://rpc.example.com', 'C123', null, { startLedger: 5000 })
    ).rejects.toThrow('RPC error (-32602): Invalid params');
  });

  test('includes pagination cursor on subsequent pages', async () => {
    const mockEvents = Array.from({ length: 250 }, (_, i) => ({
      contractId: 'C123',
      ledger: 6000 + i,
      type: 'progress_updated',
      topics: ['progress_updated', 'wallet1'],
      data: { player_id: 1 },
    }));

    global.fetch = createMockFetch(mockEvents);

    const mod = require(SCRIPT_PATH);
    const result = await mod.fetchEvents('http://rpc.example.com', 'C123', null, { startLedger: 6000 });

    expect(result.length).toBe(250);
    expect(global.fetch).toHaveBeenCalledTimes(2);

    // Second request should include cursor
    const secondRequest = JSON.parse(global.fetch.mock.calls[1][1].body);
    expect(secondRequest.params.pagination.cursor).toBeDefined();
    expect(secondRequest.params.startLedger).toBeUndefined();
  });

  test('falls back to a safe startLedger when RPC info unavailable', async () => {
    // Mock getRpcInfo to return null
    jest.doMock(SCRIPT_PATH, () => {
      const actual = jest.requireActual(SCRIPT_PATH);
      return {
        ...actual,
        getRpcInfo: jest.fn().mockResolvedValue(null),
      };
    });

    const mockEvents = Array.from({ length: 10 }, (_, i) => ({
      contractId: 'C123',
      ledger: 7000 + i,
      type: 'progress_updated',
      topics: ['progress_updated', 'wallet1'],
      data: { player_id: 1 },
    }));

    global.fetch = createMockFetch(mockEvents);

    // Force rerequire the module with mocked getRpcInfo
    jest.resetModules();
    const mod = require(SCRIPT_PATH);

    const result = await mod.fetchEvents('http://rpc.example.com', 'C123', null, { startLedger: null });

    // Should have warned and used fallback
    expect(result.length).toBe(10);
  });
});

describe('scripts/audit-event-history.js getRpcInfo', () => {
  let originalFetch;

  beforeEach(() => {
    originalFetch = global.fetch;
  });

  afterEach(() => {
    global.fetch = originalFetch;
    jest.restoreAllMocks();
  });

  test('returns latestLedger and oldestLedger from RPC responses', async () => {
    global.fetch = createRpcInfoMock({
      latestLedger: 10000,
      retention: 5000,
    });

    const mod = require(SCRIPT_PATH);
    const info = await mod.getRpcInfo('http://rpc.example.com');

    expect(global.fetch).toHaveBeenCalledTimes(2);
    expect(info).toEqual({
      latestLedger: 10000,
      oldestLedger: 5001,
    });
  });

  test('returns null when getHealth fails', async () => {
    global.fetch = jest.fn(async () => ({
      ok: false,
      status: 500,
    }));

    const mod = require(SCRIPT_PATH);
    const info = await mod.getRpcInfo('http://rpc.example.com');

    expect(info).toBeNull();
  });

  test('returns partial info when only getLatestLedger fails', async () => {
    const healthMock = jest.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        jsonrpc: '2.0',
        id: 1,
        result: { status: 'healthy' },
      }),
    }));

    const latestMock = jest.fn(async () => ({
      ok: false,
      status: 500,
    }));

    global.fetch = jest.fn().mockImplementation((url, opts) => {
      const body = JSON.parse(opts.body);
      if (body.method === 'getHealth') return healthMock(url, opts);
      return latestMock(url, opts);
    });

    const mod = require(SCRIPT_PATH);
    const info = await mod.getRpcInfo('http://rpc.example.com');

    expect(info).toEqual({
      latestLedger: null,
      oldestLedger: null,
    });
  });

  test('warns when retention info is missing', async () => {
    global.fetch = createRpcInfoMock({
      latestLedger: 10000,
      retention: null,
    });

    const mockConsoleWarn = jest.spyOn(console, 'warn').mockImplementation();

    const mod = require(SCRIPT_PATH);
    const result = await mod.fetchEvents('http://rpc.example.com', 'C123', null, {});

    // Should use fallback (1000 ledgers back from latest = 9000)
    expect(mockConsoleWarn).toHaveBeenCalledWith(
      expect.stringContaining('Could not determine RPC event retention'),
      expect.stringContaining('Starting from ledger')
    );

    mockConsoleWarn.mockRestore();
  });
});

describe('scripts/audit-event-history.js CLI parsing', () => {
  let originalFetch;

  beforeEach(() => {
    originalFetch = global.fetch;
  });

  afterEach(() => {
    global.fetch = originalFetch;
  });

  test('parseArgs handles positional player ID', async () => {
    const mod = require(SCRIPT_PATH);
    const args = mod.parseArgs(['audit-event-history.js', '42']);
    expect(args.playerIdArg).toBe('42');
  });

  test('parseArgs handles --player-id option', async () => {
    const mod = require(SCRIPT_PATH);
    const args = mod.parseArgs(['audit-event-history.js', '--player-id', '99']);
    expect(args.playerIdArg).toBe('99');
  });

  test('parseArgs handles --sample option', async () => {
    const mod = require(SCRIPT_PATH);
    const args = mod.parseArgs(['audit-event-history.js', '--sample', '10']);
    expect(args.sample).toBe(10);
  });

  test('parseArgs handles --json option', async () => {
    const mod = require(SCRIPT_PATH);
    const args = mod.parseArgs(['audit-event-history.js', '--json']);
    expect(args.json).toBe(true);
  });

  test('parseArgs handles --start-ledger option', async () => {
    const mod = require(SCRIPT_PATH);
    const args = mod.parseArgs(['audit-event-history.js', '--start-ledger', '5000']);
    expect(args.startLedger).toBe(5000);
  });

  test('parseArgs combines all options', async () => {
    const mod = require(SCRIPT_PATH);
    const args = mod.parseArgs([
      'audit-event-history.js',
      '--player-id',
      '5',
      '--sample',
      '20',
      '--json',
      '--start-ledger',
      '3000',
    ]);
    expect(args.playerIdArg).toBe('5');
    expect(args.sample).toBe(20);
    expect(args.json).toBe(true);
    expect(args.startLedger).toBe(3000);
  });

  test('parseArgs handles "all" keyword', async () => {
    const mod = require(SCRIPT_PATH);
    const args = mod.parseArgs(['audit-event-history.js', 'all']);
    expect(args.playerIdArg).toBe('all');
  });
});

describe('scripts/audit-event-history.js requireEnv', () => {
  let originalFetch;

  beforeEach(() => {
    originalFetch = global.fetch;
  });

  afterEach(() => {
    global.fetch = originalFetch;
  });

  test('requireEnv throws when env vars are missing', async () => {
    const mod = require(SCRIPT_PATH);

    // Clear any existing env vars
    const originalEnv = { ...process.env };
    delete process.env.RPC_URL;
    delete process.env.TEST_VAR;

    expect(() => {
      mod.requireEnv(['RPC_URL', 'TEST_VAR']);
    }).toThrow('Missing required environment variable(s): RPC_URL, TEST_VAR');

    // Restore
    Object.assign(process.env, originalEnv);
  });

  test('requireEnv passes when all vars exist', async () => {
    const mod = require(SCRIPT_PATH);

    process.env.RPC_URL = 'test-url';
    process.env.TEST_VAR = 'test-value';

    expect(() => {
      mod.requireEnv(['RPC_URL', 'TEST_VAR']);
    }).not.toThrow();

    delete process.env.RPC_URL;
    delete process.env.TEST_VAR;
  });
});

describe('scripts/audit-event-history.js PlayerHistoryReconstructor', () => {
  let originalFetch;

  beforeEach(() => {
    originalFetch = global.fetch;
  });

  afterEach(() => {
    global.fetch = originalFetch;
  });

  test('processProgressUpdateEvent validates level transitions', async () => {
    const mod = require(SCRIPT_PATH);
    const reconstructor = new mod.PlayerHistoryReconstructor(1);

    // Valid transition: 0 -> 1
    reconstructor.processProgressUpdateEvent({
      data: { json: JSON.stringify({ player_id: 1, old_level: 'Unverified', new_level: 'VerifiedIdentity' }) },
      ledger: 100,
      ledger_close_time: new Date(100 * 1000).toISOString(),
    });

    // Valid transition: 1 -> 2
    reconstructor.processProgressUpdateEvent({
      data: { json: JSON.stringify({ player_id: 1, old_level: 'VerifiedIdentity', new_level: 'PerformanceMilestones' }) },
      ledger: 200,
      ledger_close_time: new Date(200 * 1000).toISOString(),
    });

    // Invalid transition: 2 -> 0 (can only reset or advance by 1)
    reconstructor.processProgressUpdateEvent({
      data: { json: JSON.stringify({ player_id: 1, old_level: 'PerformanceMilestones', new_level: 'Unverified' }) },
      ledger: 300,
      ledger_close_time: new Date(300 * 1000).toISOString(),
    });

    const issues = reconstructor.getReconstructedState().issues;
    expect(issues).toHaveLength(1);
    expect(issues[0].severity).toBe('error');
    expect(issues[0].category).toBe('progress_update');
  });

  test('processPlayerLevelResetEvent handles resets', async () => {
    const mod = require(SCRIPT_PATH);
    const reconstructor = new mod.PlayerHistoryReconstructor(1);

    reconstructor.processPlayerLevelResetEvent({
      data: { json: JSON.stringify({ player_id: 1, level: 'Unverified' }) },
      ledger: 100,
      ledger_close_time: new Date(100 * 1000).toISOString(),
    });

    const state = reconstructor.getReconstructedState();
    expect(state.currentLevel).toBe(0);
  });
});
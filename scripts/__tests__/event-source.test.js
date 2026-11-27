"use strict";
/**
 * Tests for scripts/lib/event-source.js
 *
 * Uses in-memory fixtures and a mock fetch — no network calls are made.
 */

const {
  RpcEventSource,
  GalexieArchiveEventSource,
  normaliseRpcEvent,
  normaliseArchiveEvent,
  checkRangeCoverage,
  createEventSource,
} = require("../lib/event-source");

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const CONTRACT_A = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
const CONTRACT_B = "CBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

/** Minimal raw RPC event for CONTRACT_A at ledger 1000 */
const RAW_RPC_EVENT = {
  contractId: CONTRACT_A,
  type: "contract",
  ledger: "1000",
  ledger_close_time: "2024-01-01T00:00:00Z",
  value: { xdr: "AAAAAA==" },
};

/** Minimal raw archive record for CONTRACT_A at ledger 500 */
const RAW_ARCHIVE_RECORD = {
  contract_id: CONTRACT_A,
  event_type: "contract",
  ledger_sequence: 500,
  close_time: "2023-01-01T00:00:00Z",
  data: { key: "val" },
};

// ---------------------------------------------------------------------------
// normaliseRpcEvent
// ---------------------------------------------------------------------------

describe("normaliseRpcEvent", () => {
  test("maps contractId, type, ledger, ledgerCloseTime, and data", () => {
    const ev = normaliseRpcEvent(RAW_RPC_EVENT);
    expect(ev.contractId).toBe(CONTRACT_A);
    expect(ev.type).toBe("contract");
    expect(ev.ledger).toBe(1000);
    expect(ev.ledgerCloseTime).toBe("2024-01-01T00:00:00Z");
    expect(ev.data).toEqual({ xdr: "AAAAAA==" });
  });

  test("handles missing optional fields gracefully", () => {
    const ev = normaliseRpcEvent({ ledger: "42" });
    expect(ev.contractId).toBeNull();
    expect(ev.type).toBeNull();
    expect(ev.ledger).toBe(42);
    expect(ev.data).toEqual({});
  });

  test("parses data JSON string", () => {
    const ev = normaliseRpcEvent({ ledger: "1", data: '{"foo":1}' });
    expect(ev.data).toEqual({ foo: 1 });
  });
});

// ---------------------------------------------------------------------------
// normaliseArchiveEvent
// ---------------------------------------------------------------------------

describe("normaliseArchiveEvent", () => {
  test("maps contract_id, event_type, ledger_sequence, close_time, and data", () => {
    const ev = normaliseArchiveEvent(RAW_ARCHIVE_RECORD);
    expect(ev.contractId).toBe(CONTRACT_A);
    expect(ev.type).toBe("contract");
    expect(ev.ledger).toBe(500);
    expect(ev.ledgerCloseTime).toBe("2023-01-01T00:00:00Z");
    expect(ev.data).toEqual({ key: "val" });
  });

  test("falls back to camelCase field names", () => {
    const ev = normaliseArchiveEvent({
      contractId: CONTRACT_B,
      type: "event",
      ledger: "200",
      ledgerCloseTime: "t",
      data: {},
    });
    expect(ev.contractId).toBe(CONTRACT_B);
    expect(ev.ledger).toBe(200);
  });
});

// ---------------------------------------------------------------------------
// RpcEventSource
// ---------------------------------------------------------------------------

describe("RpcEventSource", () => {
  function makeRpcFetch({ events = [], cursor = null, oldestLedger = null } = {}) {
    return jest.fn(async (_url, opts) => {
      const body = JSON.parse(opts.body);
      let result;
      if (body.method === "getLatestLedger") {
        result = { sequence: 2000, oldestLedger };
      } else {
        result = { events, cursor };
      }
      return {
        json: async () => ({ jsonrpc: "2.0", id: 1, result }),
      };
    });
  }

  test("requires rpcUrl", () => {
    expect(() => new RpcEventSource({})).toThrow("rpcUrl");
  });

  test("fetch returns normalised events", async () => {
    const fetchFn = makeRpcFetch({ events: [RAW_RPC_EVENT] });
    const src = new RpcEventSource({ rpcUrl: "http://rpc", fetchFn });
    const events = await src.fetch([CONTRACT_A], 900, 1100);
    expect(events).toHaveLength(1);
    expect(events[0].contractId).toBe(CONTRACT_A);
    expect(events[0].ledger).toBe(1000);
  });

  test("paginates until page is smaller than page size", async () => {
    // First page: 200 events; second page: 1 event (signals end of stream)
    const page1 = Array.from({ length: 200 }, (_, i) => ({
      ...RAW_RPC_EVENT,
      ledger: String(1000 + i),
    }));
    const page2 = [{ ...RAW_RPC_EVENT, ledger: "1200" }];

    let call = 0;
    const fetchFn = jest.fn(async (_url, opts) => {
      const body = JSON.parse(opts.body);
      const page = call++ === 0 ? page1 : page2;
      const result = { events: page, cursor: call === 1 ? "cursor-1" : null };
      return { json: async () => ({ jsonrpc: "2.0", id: 1, result }) };
    });

    const src = new RpcEventSource({ rpcUrl: "http://rpc", fetchFn });
    const events = await src.fetch([CONTRACT_A], 1000, 1200);
    expect(events).toHaveLength(201);
  });

  test("oldestRetainedLedger returns value from getLatestLedger", async () => {
    const fetchFn = makeRpcFetch({ oldestLedger: 500 });
    const src = new RpcEventSource({ rpcUrl: "http://rpc", fetchFn });
    const oldest = await src.oldestRetainedLedger();
    expect(oldest).toBe(500);
  });

  test("oldestRetainedLedger returns null when not in response", async () => {
    const fetchFn = makeRpcFetch({ oldestLedger: null });
    const src = new RpcEventSource({ rpcUrl: "http://rpc", fetchFn });
    const oldest = await src.oldestRetainedLedger();
    expect(oldest).toBeNull();
  });

  test("throws on RPC error response", async () => {
    const fetchFn = jest.fn(async () => ({
      json: async () => ({ jsonrpc: "2.0", id: 1, error: { message: "oops" } }),
    }));
    const src = new RpcEventSource({ rpcUrl: "http://rpc", fetchFn });
    await expect(src.fetch([CONTRACT_A], 1, 100)).rejects.toThrow("oops");
  });
});

// ---------------------------------------------------------------------------
// GalexieArchiveEventSource
// ---------------------------------------------------------------------------

describe("GalexieArchiveEventSource", () => {
  /**
   * Build a mock fetch that serves:
   *   /index.json  → array of partition descriptors
   *   /<file>      → JSONL of raw archive records
   */
  function makeArchiveFetch(index, recordsByFile) {
    return jest.fn(async (url) => {
      if (url.endsWith("/index.json")) {
        return {
          ok: true,
          json: async () => index,
        };
      }
      const file = url.split("/").pop();
      const lines = (recordsByFile[file] || []).map((r) => JSON.stringify(r)).join("\n");
      return { ok: true, text: async () => lines };
    });
  }

  const INDEX = [
    { file: "part-0001.jsonl", firstLedger: 1, lastLedger: 999 },
    { file: "part-1000.jsonl", firstLedger: 1000, lastLedger: 1999 },
  ];

  const RECORDS = {
    "part-0001.jsonl": [RAW_ARCHIVE_RECORD], // ledger 500
    "part-1000.jsonl": [
      { ...RAW_ARCHIVE_RECORD, ledger_sequence: 1050, contract_id: CONTRACT_B },
      { ...RAW_ARCHIVE_RECORD, ledger_sequence: 1100 },
    ],
  };

  test("requires archiveBaseUrl", () => {
    expect(() => new GalexieArchiveEventSource({})).toThrow("archiveBaseUrl");
  });

  test("fetches and filters by contract and ledger range", async () => {
    const fetchFn = makeArchiveFetch(INDEX, RECORDS);
    const src = new GalexieArchiveEventSource({
      archiveBaseUrl: "http://archive",
      fetchFn,
    });
    // Only CONTRACT_A events in ledger range 400–1200
    const events = await src.fetch([CONTRACT_A], 400, 1200);
    expect(events.map((e) => e.ledger).sort((a, b) => a - b)).toEqual([500, 1100]);
    expect(events.every((e) => e.contractId === CONTRACT_A)).toBe(true);
  });

  test("only downloads partitions that overlap the requested range", async () => {
    const fetchFn = makeArchiveFetch(INDEX, RECORDS);
    const src = new GalexieArchiveEventSource({
      archiveBaseUrl: "http://archive",
      fetchFn,
    });
    await src.fetch([CONTRACT_A], 1000, 1200);
    // Should NOT have fetched part-0001.jsonl
    const urls = fetchFn.mock.calls.map(([url]) => url);
    expect(urls.some((u) => u.includes("part-0001"))).toBe(false);
    expect(urls.some((u) => u.includes("part-1000"))).toBe(true);
  });

  test("throws when index is not reachable", async () => {
    const fetchFn = jest.fn(async () => ({ ok: false, status: 404 }));
    const src = new GalexieArchiveEventSource({
      archiveBaseUrl: "http://archive",
      fetchFn,
    });
    await expect(src.fetch([CONTRACT_A], 1, 100)).rejects.toThrow("HTTP 404");
  });

  test("sends Authorization header when apiKey is set", async () => {
    const fetchFn = makeArchiveFetch([], {});
    const src = new GalexieArchiveEventSource({
      archiveBaseUrl: "http://archive",
      apiKey: "secret-key",
      fetchFn,
    });
    await src.fetch([CONTRACT_A], 1, 100);
    const [, opts] = fetchFn.mock.calls[0];
    expect(opts.headers["Authorization"]).toBe("Bearer secret-key");
  });
});

// ---------------------------------------------------------------------------
// checkRangeCoverage — gap detection
// ---------------------------------------------------------------------------

describe("checkRangeCoverage", () => {
  function makeRpcSource(oldestLedger) {
    return { oldestRetainedLedger: async () => oldestLedger };
  }
  const archival = { fetch: jest.fn() };

  test("returns ok when archival source is configured", async () => {
    const rpc = makeRpcSource(1000);
    const result = await checkRangeCoverage(1, 500, rpc, archival);
    expect(result.ok).toBe(true);
  });

  test("returns ok when fromLedger >= oldestRetainedLedger", async () => {
    const rpc = makeRpcSource(1000);
    const result = await checkRangeCoverage(1000, 2000, rpc, null);
    expect(result.ok).toBe(true);
  });

  test("returns ok when RPC does not expose oldest ledger", async () => {
    const rpc = makeRpcSource(null);
    const result = await checkRangeCoverage(1, 500, rpc, null);
    expect(result.ok).toBe(true);
  });

  test("returns not-ok with descriptive message when range starts before retention window", async () => {
    const rpc = makeRpcSource(1000);
    const result = await checkRangeCoverage(1, 2000, rpc, null);
    expect(result.ok).toBe(false);
    expect(result.message).toMatch(/ledger 1 but the RPC node only retains/);
    expect(result.message).toMatch(/GALEXIE_ARCHIVE_URL/);
  });
});

// ---------------------------------------------------------------------------
// createEventSource factory
// ---------------------------------------------------------------------------

describe("createEventSource", () => {
  test("throws when rpcUrl is absent", () => {
    expect(() => createEventSource({ rpcUrl: undefined })).toThrow("rpcUrl is required");
  });

  test("returns rpc source and null archival when no archive URL is set", () => {
    const { rpc, archival } = createEventSource({ rpcUrl: "http://rpc" });
    expect(rpc).toBeInstanceOf(RpcEventSource);
    expect(archival).toBeNull();
  });

  test("returns both sources when galexieArchiveUrl is provided", () => {
    const { rpc, archival } = createEventSource({
      rpcUrl: "http://rpc",
      galexieArchiveUrl: "http://archive",
    });
    expect(rpc).toBeInstanceOf(RpcEventSource);
    expect(archival).toBeInstanceOf(GalexieArchiveEventSource);
  });
});

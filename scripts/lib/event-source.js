"use strict";
/**
 * Pluggable event source abstraction for ScoutChain audit / indexer tooling.
 *
 * Interface
 * ---------
 * Every event source exposes a single async method:
 *
 *   fetch(contractIds, fromLedger, toLedger) → Promise<NormalisedEvent[]>
 *
 * where:
 *   contractIds  string[]  — Soroban contract IDs to fetch events for
 *   fromLedger   number    — inclusive start ledger sequence (genesis = 1)
 *   toLedger     number    — inclusive end ledger sequence
 *
 * A NormalisedEvent has the shape:
 *   { contractId, type, ledger, ledgerCloseTime, data }
 *
 * Gap detection
 * -------------
 * Before each fetch, callers should obtain the oldest ledger retained by the
 * RPC node via RpcEventSource.prototype.oldestRetainedLedger().  If fromLedger
 * is earlier than that value and no archival source is configured, the caller
 * must refuse to continue rather than silently producing a partial report.
 *
 * Use createEventSource(config) to obtain the appropriate source automatically.
 */

// ---------------------------------------------------------------------------
// Shared normalisation
// ---------------------------------------------------------------------------

/**
 * Normalise a raw Soroban RPC event object into our canonical shape.
 *
 * @param {object} rawEvent  — event object as returned by getEvents
 * @returns {NormalisedEvent}
 */
function normaliseRpcEvent(rawEvent) {
  return {
    contractId: rawEvent.contractId ?? rawEvent.contract_id ?? null,
    type: rawEvent.type ?? null,
    ledger: Number(rawEvent.ledger),
    ledgerCloseTime: rawEvent.ledger_close_time ?? rawEvent.ledgerCloseTime ?? null,
    data: (() => {
      try {
        if (rawEvent.value && typeof rawEvent.value.xdr === "string") {
          return { xdr: rawEvent.value.xdr };
        }
        if (rawEvent.data) {
          return typeof rawEvent.data === "string"
            ? JSON.parse(rawEvent.data)
            : rawEvent.data;
        }
        return {};
      } catch {
        return {};
      }
    })(),
  };
}

/**
 * Normalise a raw Galexie / data-lake LedgerCloseMeta event record into our
 * canonical shape.  Galexie exports group events inside transaction envelopes;
 * the archive adapter extracts and flattens them before calling this.
 *
 * @param {object} rawRecord  — flattened record from the archive
 * @returns {NormalisedEvent}
 */
function normaliseArchiveEvent(rawRecord) {
  return {
    contractId: rawRecord.contract_id ?? rawRecord.contractId ?? null,
    type: rawRecord.event_type ?? rawRecord.type ?? null,
    ledger: Number(rawRecord.ledger_sequence ?? rawRecord.ledger),
    ledgerCloseTime: rawRecord.close_time ?? rawRecord.ledgerCloseTime ?? null,
    data: rawRecord.data ?? {},
  };
}

// ---------------------------------------------------------------------------
// RpcEventSource
// ---------------------------------------------------------------------------

const RPC_PAGE_SIZE = 200;

/**
 * Fetches events directly from a Soroban RPC node.
 *
 * The RPC node only retains events for a limited window (typically a few days).
 * Use oldestRetainedLedger() to detect when the requested range starts before
 * that window.
 */
class RpcEventSource {
  /**
   * @param {object} opts
   * @param {string} opts.rpcUrl  — Soroban JSON-RPC endpoint URL
   * @param {Function} [opts.fetchFn]  — optional override for the global fetch
   *   (useful in tests)
   */
  constructor({ rpcUrl, fetchFn }) {
    if (!rpcUrl) throw new Error("RpcEventSource requires rpcUrl");
    this._rpcUrl = rpcUrl;
    this._fetch = fetchFn ?? globalThis.fetch;
  }

  /**
   * Query the RPC node for the oldest ledger sequence it still retains events
   * for.  Returns null when the RPC does not expose this information.
   *
   * @returns {Promise<number|null>}
   */
  async oldestRetainedLedger() {
    try {
      const res = await this._rpc("getLatestLedger", {});
      // Some RPC implementations include oldestLedger / protocolVersion in the
      // getLatestLedger response; use it when present.
      if (res && typeof res.oldestLedger === "number") {
        return res.oldestLedger;
      }
      return null;
    } catch {
      return null;
    }
  }

  /**
   * Fetch normalised events for the given contracts and ledger range.
   *
   * @param {string[]} contractIds
   * @param {number}   fromLedger
   * @param {number}   toLedger
   * @returns {Promise<NormalisedEvent[]>}
   */
  async fetch(contractIds, fromLedger, toLedger) {
    const all = [];
    let cursor = undefined;

    while (true) {
      const params = {
        contractIds,
        limit: RPC_PAGE_SIZE,
        startLedger: fromLedger,
      };
      if (cursor !== undefined) params.cursor = cursor;
      if (toLedger != null) params.endLedger = toLedger;

      const result = await this._rpc("getEvents", params);
      const page = (result && result.events) || [];

      for (const ev of page) {
        all.push(normaliseRpcEvent(ev));
      }

      if (page.length < RPC_PAGE_SIZE || !result.cursor) break;
      cursor = result.cursor;
    }

    return all;
  }

  /** @private */
  async _rpc(method, params) {
    const res = await this._fetch(this._rpcUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    });
    const body = await res.json();
    if (body.error) throw new Error(`RPC ${method} error: ${body.error.message}`);
    return body.result;
  }
}

// ---------------------------------------------------------------------------
// GalexieArchiveEventSource
// ---------------------------------------------------------------------------

/**
 * Fetches events from a Galexie / Stellar data-lake archival export.
 *
 * Galexie exports LedgerCloseMeta in JSONL files partitioned by ledger range
 * and uploaded to a GCS/S3 bucket (or reachable via HTTP).  This source
 * fetches the index manifest from ARCHIVE_BASE_URL/index.json, identifies
 * which partition files overlap with [fromLedger, toLedger], downloads them,
 * and extracts the matching contract events.
 *
 * Configuration (environment variables):
 *   GALEXIE_ARCHIVE_URL   — base URL of the archive  (required)
 *   GALEXIE_API_KEY       — optional Bearer token for authenticated buckets
 *
 * The archive is assumed to be an HTTP(S) endpoint that serves:
 *   <base>/index.json              — partition index
 *   <base>/<partition-file>.jsonl  — newline-delimited event records
 *
 * Each record in a partition file must have at minimum:
 *   ledger_sequence, contract_id, event_type, data
 *
 * If the partition file is not reachable this source throws so the caller can
 * handle the gap explicitly rather than silently producing partial results.
 */
class GalexieArchiveEventSource {
  /**
   * @param {object} opts
   * @param {string}   opts.archiveBaseUrl  — base URL of the Galexie archive
   * @param {string}   [opts.apiKey]        — optional Bearer token
   * @param {Function} [opts.fetchFn]       — optional override for the global fetch
   */
  constructor({ archiveBaseUrl, apiKey, fetchFn }) {
    if (!archiveBaseUrl) {
      throw new Error("GalexieArchiveEventSource requires archiveBaseUrl");
    }
    this._base = archiveBaseUrl.replace(/\/$/, "");
    this._apiKey = apiKey ?? null;
    this._fetch = fetchFn ?? globalThis.fetch;
  }

  /**
   * Fetch normalised events from the archive for [fromLedger, toLedger].
   *
   * @param {string[]} contractIds
   * @param {number}   fromLedger
   * @param {number}   toLedger
   * @returns {Promise<NormalisedEvent[]>}
   */
  async fetch(contractIds, fromLedger, toLedger) {
    const index = await this._fetchIndex();
    const partitions = this._selectPartitions(index, fromLedger, toLedger);
    const contractSet = new Set(contractIds);
    const all = [];

    for (const partition of partitions) {
      const records = await this._fetchPartition(partition.file);
      for (const rec of records) {
        const ledger = Number(rec.ledger_sequence ?? rec.ledger);
        if (ledger < fromLedger || ledger > toLedger) continue;
        if (!contractSet.has(rec.contract_id ?? rec.contractId)) continue;
        all.push(normaliseArchiveEvent(rec));
      }
    }

    return all;
  }

  /** @private — fetch and parse the partition index */
  async _fetchIndex() {
    const url = `${this._base}/index.json`;
    const res = await this._fetchWithAuth(url);
    if (!res.ok) {
      throw new Error(
        `GalexieArchiveEventSource: failed to fetch index from ${url} — HTTP ${res.status}`,
      );
    }
    return res.json();
  }

  /**
   * @private — select partition entries whose ledger range overlaps [from, to]
   * @param {Array<{file: string, firstLedger: number, lastLedger: number}>} index
   */
  _selectPartitions(index, fromLedger, toLedger) {
    return index.filter(
      (p) => p.lastLedger >= fromLedger && p.firstLedger <= toLedger,
    );
  }

  /** @private — fetch and parse a single JSONL partition file */
  async _fetchPartition(file) {
    const url = `${this._base}/${file}`;
    const res = await this._fetchWithAuth(url);
    if (!res.ok) {
      throw new Error(
        `GalexieArchiveEventSource: failed to fetch partition ${url} — HTTP ${res.status}`,
      );
    }
    const text = await res.text();
    return text
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        try {
          return JSON.parse(line);
        } catch {
          return null;
        }
      })
      .filter(Boolean);
  }

  /** @private */
  async _fetchWithAuth(url) {
    const headers = {};
    if (this._apiKey) headers["Authorization"] = `Bearer ${this._apiKey}`;
    return this._fetch(url, { headers });
  }
}

// ---------------------------------------------------------------------------
// Gap detection
// ---------------------------------------------------------------------------

/**
 * Check whether the requested ledger range is fully covered.
 *
 * Returns `{ ok: true }` when the range is fully covered.
 * Returns `{ ok: false, message: string }` when it is not.
 *
 * Rules:
 *  - If an archival source is configured it is assumed to cover the full
 *    genesis-to-present range.
 *  - If only an RPC source is available, the range must start at or after
 *    the oldest retained ledger.  When the RPC does not expose oldest-ledger
 *    metadata the check passes (we cannot detect the gap).
 *
 * @param {number}            fromLedger
 * @param {number}            toLedger
 * @param {RpcEventSource}    rpcSource
 * @param {object|null}       archivalSource   — any object with a fetch method,
 *                                               or null if none is configured
 * @returns {Promise<{ok: boolean, message?: string}>}
 */
async function checkRangeCoverage(fromLedger, toLedger, rpcSource, archivalSource) {
  if (archivalSource) {
    // Archival source covers full history.
    return { ok: true };
  }

  const oldest = await rpcSource.oldestRetainedLedger();
  if (oldest === null) {
    // Cannot determine retention window; allow to proceed with a warning.
    return { ok: true };
  }

  if (fromLedger < oldest) {
    return {
      ok: false,
      message:
        `Requested range starts at ledger ${fromLedger} but the RPC node only ` +
        `retains events from ledger ${oldest}. ` +
        `Ledgers ${fromLedger}–${oldest - 1} are outside the retention window. ` +
        `Configure an archival event source (GALEXIE_ARCHIVE_URL) to cover ` +
        `the full range, or start from ledger ${oldest}.`,
    };
  }

  return { ok: true };
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/**
 * Build the appropriate event source(s) from environment / explicit config.
 *
 * Returns `{ rpc, archival }` where `archival` may be null.
 *
 * Config keys (all optional except rpcUrl):
 *   rpcUrl            — Soroban RPC URL (required)
 *   galexieArchiveUrl — base URL of Galexie archive; if omitted, checks
 *                       process.env.GALEXIE_ARCHIVE_URL
 *   galexieApiKey     — optional API key; if omitted, checks
 *                       process.env.GALEXIE_API_KEY
 *   fetchFn           — optional fetch override (for tests)
 *
 * @param {object} config
 * @returns {{ rpc: RpcEventSource, archival: GalexieArchiveEventSource|null }}
 */
function createEventSource(config = {}) {
  const {
    rpcUrl = process.env.RPC_URL,
    galexieArchiveUrl = process.env.GALEXIE_ARCHIVE_URL,
    galexieApiKey = process.env.GALEXIE_API_KEY,
    fetchFn,
  } = config;

  if (!rpcUrl) {
    throw new Error("createEventSource: rpcUrl is required (or set RPC_URL env var)");
  }

  const rpc = new RpcEventSource({ rpcUrl, fetchFn });
  const archival = galexieArchiveUrl
    ? new GalexieArchiveEventSource({ archiveBaseUrl: galexieArchiveUrl, apiKey: galexieApiKey, fetchFn })
    : null;

  return { rpc, archival };
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  RpcEventSource,
  GalexieArchiveEventSource,
  normaliseRpcEvent,
  normaliseArchiveEvent,
  checkRangeCoverage,
  createEventSource,
};

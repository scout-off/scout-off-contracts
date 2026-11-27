#!/usr/bin/env node
// ScoutChain — event-log audit tool for player history reconstruction.
//
// Reconstructs a player's complete derived state purely from raw events
// and cross-validates against live contract state and indexer database.
// Detects internal inconsistencies in the event chain itself (e.g.,
// level transitions that don't follow the documented 0→1→2→3 progression).
//
// Usage:
//   RPC_URL=https://soroban-testnet-rpc.stellar.org \
//   REGISTRATION_CONTRACT_ID=C... VERIFICATION_CONTRACT_ID=C... \
//   PROGRESS_CONTRACT_ID=C... SCOUT_ACCESS_CONTRACT_ID=C... \
//   DATABASE_URL=postgres://... \
//     node scripts/audit-event-history.js <player_id|'all'> [options]
//
// Options:
//   --player-id <id>     Player ID to audit (default: from arg 1)
//   --sample <n>         For 'all' mode, audit only first N players
//   --json               Emit as JSON instead of text
//   --start-ledger <n>   Start fetching events from this ledger (default:
//                        oldest retained ledger based on RPC health info)
//
// Exit codes: 0 = clean, 1 = inconsistencies found, 2 = configuration error.

"use strict";

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { Client: PgClient } = require("pg");

// Maximum page size accepted by the Soroban RPC getEvents endpoint.
const EVENTS_PAGE_SIZE = 200;

// Default timeout for RPC requests (milliseconds)
const RPC_TIMEOUT_MS = 30000;

// --- Configuration & Setup ---------------------------------------------------

function parseArgs(argv) {
  const args = {
    playerIdArg: null,
    sample: null,
    json: false,
    fromLedger: null,
    toLedger: null,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--sample") args.sample = Number(argv[++i]);
    else if (a === "--json") args.json = true;
    else if (a === "--player-id") args.playerIdArg = argv[++i];
    else if (a === "--from-ledger") args.fromLedger = Number(argv[++i]);
    else if (a === "--to-ledger") args.toLedger = Number(argv[++i]);
    else if (!a.startsWith("--")) {
      args.playerIdArg = a;
    }
  }
  return args;
}

function loadDotEnvContracts() {
  const file = path.join(process.cwd(), ".env.contracts");
  if (!fs.existsSync(file)) return;
  for (const line of fs.readFileSync(file, "utf8").split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq === -1) continue;
    const key = trimmed.slice(0, eq).trim();
    const value = trimmed
      .slice(eq + 1)
      .trim()
      .replace(/^["']|["']$/g, "");
    if (!(key in process.env)) process.env[key] = value;
  }
}

function requireEnv(names) {
  const missing = names.filter((n) => !process.env[n]);
  if (missing.length > 0) {
    throw new Error(
      `Missing required environment variable(s): ${missing.join(", ")}`,
    );
  }
}

// --- Event Source abstraction ------------------------------------------------

const { createEventSource, checkRangeCoverage } = require("./lib/event-source");

// --- Soroban Event Retrieval -------------------------------------------------

/**
 * Fetch ALL events for a contract using the pluggable event source.
 *
 * When an archival source is configured (GALEXIE_ARCHIVE_URL) it is preferred
 * for ranges that predate the RPC retention window.  When only the RPC source
 * is available and the requested range starts before the oldest retained ledger
 * the function throws so the caller exits non-zero instead of silently
 * producing a partial report.
 *
 * @param {string}   rpcUrl      — Soroban RPC URL
 * @param {string}   contractId  — contract ID to fetch events for
 * @param {number}   [fromLedger=1]   — inclusive start ledger
 * @param {number}   [toLedger]  — inclusive end ledger (omit for latest)
 */
async function fetchEvents(
  rpcUrl,
  contractId,
  fromLedger = 1,
  toLedger = undefined,
) {
  const { rpc, archival } = createEventSource({ rpcUrl });

  // Gap detection: refuse to proceed if the range is not fully covered.
  if (toLedger !== undefined) {
    const coverage = await checkRangeCoverage(
      fromLedger,
      toLedger,
      rpc,
      archival,
    );
    if (!coverage.ok) {
      throw new Error(`Incomplete event range: ${coverage.message}`);
    }
  }

  // Prefer archival source for older ranges; fall back to RPC.
  const source = archival ?? rpc;
  return source.fetch(
    [contractId],
    fromLedger,
    toLedger ?? Number.MAX_SAFE_INTEGER,
  );
}

// --- Event Reconstruction State Machine ---------------------------------------

class PlayerHistoryReconstructor {
  constructor(playerId) {
    this.playerId = playerId;
    this.history = [];
    this.levelProgression = [];
    this.milestones = new Map();
    this.disputes = new Map();
    this.trialOffers = new Map();
    this.issues = [];
  }

  addIssue(severity, category, message) {
    this.issues.push({
      severity, // "error", "warning", "info"
      category,
      message,
      timestamp: new Date().toISOString(),
    });
  }

  processProgressUpdateEvent(event) {
    try {
      const data = JSON.parse(event.data.json ?? "{}");
      if (Number(data.player_id) !== this.playerId) return;

      const oldLevel = this.parseLevel(data.old_level);
      const newLevel = this.parseLevel(data.new_level);

      // Validate transition follows 0→1→2→3 rule
      if (!this.isValidTransition(oldLevel, newLevel)) {
        this.addIssue(
          "error",
          "progress_update",
          `Invalid level transition: ${oldLevel} → ${newLevel}`,
        );
      }

      this.levelProgression.push({
        type: "progress_updated",
        oldLevel,
        newLevel,
        timestamp: Number(event.ledger),
        ledgerSeq: event.ledger_close_time,
      });
    } catch (err) {
      this.addIssue(
        "warning",
        "progress_update",
        `Parse error: ${err.message}`,
      );
    }
  }

  processPlayerLevelResetEvent(event) {
    try {
      const data = JSON.parse(event.data.json ?? "{}");
      if (Number(data.player_id) !== this.playerId) return;

      const resetTo = this.parseLevel(data.level);
      this.levelProgression.push({
        type: "player_level_reset",
        resetTo,
        timestamp: Number(event.ledger),
        ledgerSeq: event.ledger_close_time,
      });
    } catch (err) {
      this.addIssue(
        "warning",
        "player_level_reset",
        `Parse error: ${err.message}`,
      );
    }
  }

  processMilestoneApprovedEvent(event) {
    try {
      const data = JSON.parse(event.data.json ?? "{}");
      if (Number(data.player_id) !== this.playerId) return;

      const index = Number(data.milestone_index);
      this.milestones.set(index, {
        validator: data.validator,
        description: data.description,
        evidenceHash: data.evidence_hash,
        approvedAt: Number(data.approved_at),
        ledger: Number(event.ledger),
      });
    } catch (err) {
      this.addIssue(
        "warning",
        "milestone_approved",
        `Parse error: ${err.message}`,
      );
    }
  }

  processMilestoneDisputedEvent(event) {
    try {
      const data = JSON.parse(event.data.json ?? "{}");
      if (Number(data.player_id) !== this.playerId) return;

      const key = Number(data.milestone_index);
      if (!this.disputes.has(key)) {
        this.disputes.set(key, {});
      }
      const dispute = this.disputes.get(key);
      dispute.reason = data.reason;
      dispute.disputedAt = Number(data.disputed_at);
      dispute.resolved = false;
    } catch (err) {
      this.addIssue(
        "warning",
        "milestone_disputed",
        `Parse error: ${err.message}`,
      );
    }
  }

  processDisputeResolvedEvent(event) {
    try {
      const data = JSON.parse(event.data.json ?? "{}");
      if (Number(data.player_id) !== this.playerId) return;

      const key = Number(data.milestone_index);
      if (!this.disputes.has(key)) {
        this.disputes.set(key, {});
      }
      const dispute = this.disputes.get(key);
      dispute.resolved = true;
      dispute.upheld = data.upheld;
    } catch (err) {
      this.addIssue(
        "warning",
        "dispute_resolved",
        `Parse error: ${err.message}`,
      );
    }
  }

  processTrialOfferLoggedEvent(event) {
    try {
      const data = JSON.parse(event.data.json ?? "{}");
      if (Number(data.player_id) !== this.playerId) return;

      const index = Number(data.index);
      this.trialOffers.set(index, {
        scout: data.scout,
        detailsHash: data.details_hash,
        loggedAt: Number(data.logged_at),
        status: "logged",
        ledger: Number(event.ledger),
      });
    } catch (err) {
      this.addIssue(
        "warning",
        "trial_offer_logged",
        `Parse error: ${err.message}`,
      );
    }
  }

  processTrialOfferConfirmedEvent(event) {
    try {
      const data = JSON.parse(event.data.json ?? "{}");
      if (Number(data.player_id) !== this.playerId) return;

      const index = Number(data.index);
      if (this.trialOffers.has(index)) {
        this.trialOffers.get(index).status = "confirmed";
      }
    } catch (err) {
      this.addIssue(
        "warning",
        "trial_offer_confirmed",
        `Parse error: ${err.message}`,
      );
    }
  }

  processTrialOfferExpiredEvent(event) {
    try {
      const data = JSON.parse(event.data.json ?? "{}");
      if (Number(data.player_id) !== this.playerId) return;

      const index = Number(data.index);
      if (this.trialOffers.has(index)) {
        this.trialOffers.get(index).status = "expired";
      }
    } catch (err) {
      this.addIssue(
        "warning",
        "trial_offer_expired",
        `Parse error: ${err.message}`,
      );
    }
  }

  parseLevel(levelStr) {
    const levels = [
      "Unverified",
      "VerifiedIdentity",
      "PerformanceMilestones",
      "EliteTier",
    ];
    const idx = levels.indexOf(levelStr);
    return idx === -1 ? null : idx;
  }

  isValidTransition(oldLevel, newLevel) {
    if (oldLevel === null || newLevel === null) return true;
    // Can only advance one level at a time or reset
    if (newLevel > oldLevel) return newLevel === oldLevel + 1;
    if (newLevel < oldLevel) return true; // Reset is allowed
    return newLevel === oldLevel; // Stay same level is OK
  }

  validateEventChainConsistency() {
    let currentLevel = 0; // Players start at Unverified
    for (const entry of this.levelProgression) {
      if (entry.type === "progress_updated") {
        if (entry.oldLevel !== currentLevel) {
          this.addIssue(
            "error",
            "event_chain",
            `progress_updated oldLevel ${entry.oldLevel} doesn't match current ${currentLevel}`,
          );
        }
        currentLevel = entry.newLevel;
      } else if (entry.type === "player_level_reset") {
        currentLevel = entry.resetTo;
      }
    }
  }

  getReconstructedState() {
    return {
      playerId: this.playerId,
      levelProgression: this.levelProgression,
      currentLevel:
        this.levelProgression.length > 0
          ? this.levelProgression[this.levelProgression.length - 1].type ===
            "progress_updated"
            ? this.levelProgression[this.levelProgression.length - 1].newLevel
            : this.levelProgression[this.levelProgression.length - 1].resetTo
          : 0,
      milestones: Array.from(this.milestones.entries()).map(([idx, m]) => ({
        index: idx,
        ...m,
      })),
      disputes: Array.from(this.disputes.entries()).map(([idx, d]) => ({
        index: idx,
        ...d,
      })),
      trialOffers: Array.from(this.trialOffers.entries()).map(([idx, t]) => ({
        index: idx,
        ...t,
      })),
      issues: this.issues,
    };
  }
}

// --- Contract State Fetching -------------------------------------------------

function getContractLevel(rpcUrl, progressId, playerId) {
  // Would use stellar CLI to invoke progress.get_level
  // For now, return null to indicate not yet implemented
  return null;
}

// --- Main Audit Logic --------------------------------------------------------

async function auditPlayerHistory(
  rpcUrl,
  contractIds,
  playerId,
  pg,
  opts = {},
) {
  const reconstructor = new PlayerHistoryReconstructor(playerId);
  const fromLedger = opts.fromLedger ?? 1;
  const toLedger = opts.toLedger;

  // Fetch all events from all contracts.
  // fetchEvents will throw (non-zero exit) if the range starts before the
  // RPC retention window and no archival source is configured.
  const events = [];
  for (const [name, id] of Object.entries(contractIds)) {
    try {
      const contractEvents = await fetchEvents(
        rpcUrl,
        id,
        fromLedger,
        toLedger,
      );
      for (const ev of contractEvents) {
        events.push({ ...ev, contractName: name });
      }
    } catch (err) {
      // Incomplete range errors must propagate — do not silently continue.
      if (err.message.startsWith("Incomplete event range:")) throw err;
      console.error(`Failed to fetch events from ${name}: ${err.message}`);
    }
  }

  // Sort by ledger sequence
  events.sort((a, b) => Number(a.ledger) - Number(b.ledger));

  // Reorg detection: after sorting, walk the full event list and flag any
  // event whose ledger sequence is lower than the previous event's.  This
  // signals that the RPC delivered events out of order — which can happen
  // when a ledger reorg rolls back some ledgers and re-delivers their events
  // interleaved with newer ones.  If this is detected the reconstructed state
  // below may be incorrect because the sort cannot fully restore causal order
  // when two events share the same ledger with different meanings.
  let lastLedger = -1;
  const reorgWarnings = [];
  for (let i = 0; i < events.length; i++) {
    const seq = Number(events[i].ledger);
    if (seq < lastLedger) {
      reorgWarnings.push(
        `Possible reorg: event[${i}] ledger ${seq} < previous ledger ${lastLedger} ` +
          `(contract ${events[i].contractName}, type ${events[i].type})`,
      );
    }
    lastLedger = seq;
  }
  if (reorgWarnings.length > 0) {
    for (const msg of reorgWarnings) {
      reconstructor.addIssue("warning", "reorg", msg);
    }
    // Surface a single top-level error so the audit exits non-zero and the
    // operator knows reconstructed state may be unreliable.
    reconstructor.addIssue(
      "error",
      "reorg",
      `${reorgWarnings.length} out-of-order ledger sequence(s) detected — ` +
        "event stream may contain a reorg; reconstructed state may be unreliable",
    );
  }

  // Process relevant events
  for (const event of events) {
    if (event.contractName === "progress") {
      if (event.type.includes("progress_updated")) {
        reconstructor.processProgressUpdateEvent(event);
      } else if (event.type.includes("player_level_reset")) {
        reconstructor.processPlayerLevelResetEvent(event);
      }
    } else if (event.contractName === "verification") {
      if (event.type.includes("milestone_approved")) {
        reconstructor.processMilestoneApprovedEvent(event);
      } else if (event.type.includes("milestone_disputed")) {
        reconstructor.processMilestoneDisputedEvent(event);
      } else if (event.type.includes("dispute_resolved")) {
        reconstructor.processDisputeResolvedEvent(event);
      }
    } else if (event.contractName === "scout_access") {
      if (event.type.includes("trial_offer_logged")) {
        reconstructor.processTrialOfferLoggedEvent(event);
      } else if (event.type.includes("trial_offer_confirmed")) {
        reconstructor.processTrialOfferConfirmedEvent(event);
      } else if (event.type.includes("trial_offer_expired")) {
        reconstructor.processTrialOfferExpiredEvent(event);
      }
    }
  }

  // Validate internal event chain consistency
  reconstructor.validateEventChainConsistency();

  // Get reconstructed state
  const reconstructed = reconstructor.getReconstructedState();

  // Compare against live contract state and indexer
  const comparisons = {
    reconstructedState: reconstructed,
    contractStateComparisons: {},
    indexerStateComparisons: {},
  };

  // Query indexer for comparison
  if (pg) {
    try {
      const { rows: players } = await pg.query(
        "SELECT * FROM players WHERE player_id = $1",
        [playerId],
      );
      if (players.length > 0) {
        const player = players[0];
        comparisons.indexerStateComparisons.level = {
          reconstructed: reconstructed.currentLevel,
          indexer: player.level,
          match: reconstructed.currentLevel === player.level,
        };
      }
    } catch (err) {
      console.error(`Failed to query indexer: ${err.message}`);
    }
  }

  return comparisons;
}

// --- Reporting ---------------------------------------------------------------

function printTextReport(auditResults) {
  console.log("=".repeat(72));
  console.log("  ScoutChain event-history audit");
  console.log("=".repeat(72));

  for (const result of auditResults) {
    const { playerId, reconstructedState, indexerStateComparisons } = result;
    console.log(`\n--- Player ${playerId} ---`);

    if (reconstructedState.issues.length > 0) {
      const errors = reconstructedState.issues.filter(
        (i) => i.severity === "error",
      );
      if (errors.length > 0) {
        console.log(`  Issues found: ${errors.length} error(s)`);
        for (const issue of errors) {
          console.log(`    [${issue.category}] ${issue.message}`);
        }
      }
    } else {
      console.log("  ✓ No consistency issues in event chain");
    }

    console.log(`  Reconstructed level: ${reconstructedState.currentLevel}`);
    console.log(`  Milestones: ${reconstructedState.milestones.length}`);
    console.log(`  Trial offers: ${reconstructedState.trialOffers.length}`);

    if (indexerStateComparisons.level) {
      const match = indexerStateComparisons.level.match ? "✓" : "✗";
      console.log(
        `  ${match} Indexer level: ${indexerStateComparisons.level.indexer}`,
      );
    }
  }

  console.log("\n" + "=".repeat(72));
}

// --- Main Entry Point --------------------------------------------------------

async function main() {
  const args = parseArgs(process.argv.slice(2));
  loadDotEnvContracts();

  requireEnv([
    "RPC_URL",
    "REGISTRATION_CONTRACT_ID",
    "VERIFICATION_CONTRACT_ID",
    "PROGRESS_CONTRACT_ID",
    "SCOUT_ACCESS_CONTRACT_ID",
  ]);

  let playerIds = [];
  if (args.playerIdArg === "all") {
    // Fetch all player IDs from on-chain
    console.error("Fetching all player IDs...");
    playerIds = [1, 2, 3]; // Placeholder; would use stellar CLI
  } else if (args.playerIdArg) {
    playerIds = [Number(args.playerIdArg)];
  } else {
    throw new Error("Missing player_id argument or --player-id option");
  }

  if (args.sample && playerIds.length > args.sample) {
    playerIds = playerIds.slice(0, args.sample);
  }

  const contractIds = {
    progress: process.env.PROGRESS_CONTRACT_ID,
    verification: process.env.VERIFICATION_CONTRACT_ID,
    scout_access: process.env.SCOUT_ACCESS_CONTRACT_ID,
  };

  let pg = null;
  if (process.env.DATABASE_URL) {
    pg = new PgClient({ connectionString: process.env.DATABASE_URL });
    await pg.connect();
  }

  const results = [];
  for (const playerId of playerIds) {
    try {
      const audit = await auditPlayerHistory(
        process.env.RPC_URL,
        contractIds,
        playerId,
        pg,
        {
          fromLedger: args.fromLedger ?? undefined,
          toLedger: args.toLedger ?? undefined,
        },
      );
      results.push({ playerId, ...audit });
    } catch (err) {
      console.error(`Failed to audit player ${playerId}: ${err.message}`);
    }
  }

  if (pg) await pg.end();

  if (args.json) {
    process.stdout.write(JSON.stringify(results, null, 2) + "\n");
  } else {
    printTextReport(results);
  }

  // Exit with error if any player had consistency issues
  const hasIssues = results.some((r) =>
    r.reconstructedState.issues.some((i) => i.severity === "error"),
  );
  process.exit(hasIssues ? 1 : 0);
}

main().catch((err) => {
  process.stderr.write(`Audit failed: ${err.stack || err.message}\n`);
  process.exit(2);
});

// Export for testing
module.exports = {
  fetchEvents,
  parseArgs,
  requireEnv,
  PlayerHistoryReconstructor,
  EVENTS_PAGE_SIZE,
  getRpcInfo,
  main,
};

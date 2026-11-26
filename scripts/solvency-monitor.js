#!/usr/bin/env node
// ScoutChain — off-chain solvency monitor for the scout_access contract.
//
// The scout_access contract holds real XLM. Its solvency invariant is:
//
//   token.balance(contract) >= AccumulatedFees + Σ outstanding TrialEscrow.amount
//
// Any deficit means the contract cannot pay out all obligations; any
// unexplained surplus (beyond a small tolerance) may indicate a bookkeeping
// regression. This script reads on-chain state via `stellar contract invoke`
// (simulation only — no transactions), builds a report, and exits non-zero
// when the invariant is violated.
//
// Usage:
//   SCOUT_ACCESS_CONTRACT_ID=C... \
//   RPC_URL=https://soroban-testnet.stellar.org \
//     node scripts/solvency-monitor.js [options]
//
// Options:
//   --network <name>     Network name passed to `stellar contract invoke` (default: testnet)
//   --rpc-url <url>      RPC URL for direct balance query via SAC simulation
//   --source <identity>  --source passed to `stellar contract invoke`
//   --json               Emit report as JSON instead of human-readable text
//   --webhook-url <url>  POST a JSON alert payload here on deficit or error
//   --tolerance <n>      Surplus tolerance in stroops before alerting (default: 0)
//
// Environment variables (all optional — CLI flags take precedence):
//   SCOUT_ACCESS_CONTRACT_ID  contract ID of scout_access
//   RPC_URL                   Soroban RPC endpoint
//   STELLAR_NETWORK           network name (default: testnet)
//   WEBHOOK_URL               Slack/Discord/generic webhook for alerts
//   SOLVENCY_SURPLUS_TOLERANCE_STROOPS
//
// Exit codes: 0 = solvent, 1 = deficit or alert condition, 2 = config/RPC error.
//
// See docs/RUNBOOK.md for how to respond to a reported deficit or surplus.

"use strict";

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const https = require("node:https");
const http = require("node:http");

// ---------------------------------------------------------------------------
// Argument / environment parsing
// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const args = {
    network: process.env.STELLAR_NETWORK || "testnet",
    rpcUrl: process.env.RPC_URL || null,
    source: null,
    json: false,
    webhookUrl: process.env.WEBHOOK_URL || null,
    toleranceStroops: BigInt(
      process.env.SOLVENCY_SURPLUS_TOLERANCE_STROOPS || "0"
    ),
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--network") args.network = argv[++i];
    else if (a === "--rpc-url") args.rpcUrl = argv[++i];
    else if (a === "--source") args.source = argv[++i];
    else if (a === "--json") args.json = true;
    else if (a === "--webhook-url") args.webhookUrl = argv[++i];
    else if (a === "--tolerance")
      args.toleranceStroops = BigInt(argv[++i] || "0");
    else {
      process.stderr.write(`Unknown argument: ${a}\n`);
      process.exit(2);
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

class ConfigError extends Error {}
class RpcError extends Error {}

// ---------------------------------------------------------------------------
// stellar contract invoke wrapper
// ---------------------------------------------------------------------------

/**
 * Invoke a read-only contract function via `stellar contract invoke` and
 * parse its JSON result. Returns { ok: true, value } on success or
 * { ok: false, error } on failure. Identical signature to reconcile-indexer.
 */
function invokeCli(network, source, contractId, fn, args) {
  const cmd = ["contract", "invoke", "--id", contractId, "--network", network];
  if (source) cmd.push("--source", source);
  cmd.push("--", fn, ...args);
  try {
    const raw = execFileSync("stellar", cmd, {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      maxBuffer: 16 * 1024 * 1024,
    });
    const trimmed = raw.trim();
    return { ok: true, value: trimmed.length ? JSON.parse(trimmed) : null };
  } catch (err) {
    const stderr = err.stderr ? err.stderr.toString() : "";
    return { ok: false, error: stderr.trim() || err.message };
  }
}

// Swappable for tests.
let invoke = invokeCli;

// ---------------------------------------------------------------------------
// On-chain data readers
// ---------------------------------------------------------------------------

/**
 * Fetch the XLM token address stored in the contract (get_xlm_token).
 * Returns the SAC address string, or throws ConfigError.
 */
async function fetchTokenAddress(cfg) {
  const r = await invoke(cfg.network, cfg.source, cfg.contractId, "get_xlm_token", []);
  if (!r.ok || r.value === null || r.value === undefined) {
    throw new ConfigError(
      `get_xlm_token failed or returned null: ${r.error || "null"}`
    );
  }
  return r.value;
}

/**
 * Read the contract's XLM balance by calling balance() on the SAC.
 * Returns a BigInt in stroops.
 */
async function fetchContractBalance(cfg, tokenId) {
  const r = await invoke(
    cfg.network,
    cfg.source,
    tokenId,
    "balance",
    ["--id", cfg.contractId]
  );
  if (!r.ok) {
    throw new RpcError(`balance() failed: ${r.error}`);
  }
  return BigInt(r.value || 0);
}

/**
 * Read AccumulatedFees from the contract.
 * Returns a BigInt in stroops.
 */
async function fetchAccumulatedFees(cfg) {
  const r = await invoke(
    cfg.network,
    cfg.source,
    cfg.contractId,
    "get_accumulated_fees",
    []
  );
  if (!r.ok) {
    throw new RpcError(`get_accumulated_fees failed: ${r.error}`);
  }
  return BigInt(r.value || 0);
}

/**
 * Page through all entries in OutstandingTrialEscrows and return the
 * total escrowed amount as a BigInt in stroops.
 *
 * Uses get_outstanding_escrows_page(offset, limit=50) in a loop.
 */
async function fetchTotalOutstandingEscrow(cfg) {
  let total = BigInt(0);
  let offset = 0;
  const limit = 50;
  let iterations = 0;
  const MAX_ITERATIONS = 10_000; // safety bound

  while (iterations++ < MAX_ITERATIONS) {
    const r = await invoke(
      cfg.network,
      cfg.source,
      cfg.contractId,
      "get_outstanding_escrows_page",
      ["--offset", String(offset), "--limit", String(limit)]
    );
    if (!r.ok) {
      throw new RpcError(`get_outstanding_escrows_page failed: ${r.error}`);
    }
    // Response: [entries, more] where entries: [(player_id, index, amount)]
    const [entries, more] = r.value;
    for (const entry of entries) {
      // entry is [player_id, index, amount] or {0: player_id, 1: index, 2: amount}
      const amount = Array.isArray(entry) ? entry[2] : entry["2"];
      total += BigInt(amount || 0);
    }
    if (!more) break;
    offset += entries.length;
  }

  return total;
}

// ---------------------------------------------------------------------------
// Alert dispatch
// ---------------------------------------------------------------------------

function postWebhook(url, payload) {
  return new Promise((resolve) => {
    const body = JSON.stringify(payload);
    const parsed = new URL(url);
    const lib = parsed.protocol === "https:" ? https : http;
    const req = lib.request(
      {
        hostname: parsed.hostname,
        port: parsed.port || (parsed.protocol === "https:" ? 443 : 80),
        path: parsed.pathname + parsed.search,
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
        },
      },
      (res) => {
        res.resume();
        resolve(res.statusCode);
      }
    );
    req.on("error", () => resolve(null));
    req.write(body);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// Report building
// ---------------------------------------------------------------------------

/**
 * Build the solvency report object. All amounts are BigInt stroops.
 */
function buildReport({
  contractId,
  network,
  balance,
  accumulatedFees,
  totalEscrow,
  toleranceStroops,
  error,
}) {
  if (error) {
    return {
      ok: false,
      status: "error",
      contractId,
      network,
      error: error.message || String(error),
      checkedAt: new Date().toISOString(),
    };
  }

  const obligations = accumulatedFees + totalEscrow;
  const delta = balance - obligations; // positive = surplus, negative = deficit

  let status;
  if (delta < 0n) {
    status = "deficit";
  } else if (delta > toleranceStroops) {
    status = "surplus";
  } else {
    status = "ok";
  }

  return {
    ok: status === "ok",
    status,
    contractId,
    network,
    balanceStroops: balance.toString(),
    accumulatedFeesStroops: accumulatedFees.toString(),
    totalOutstandingEscrowStroops: totalEscrow.toString(),
    obligationsStroops: obligations.toString(),
    deltaStroops: delta.toString(),
    toleranceStroops: toleranceStroops.toString(),
    checkedAt: new Date().toISOString(),
  };
}

function formatReport(report, useJson) {
  if (useJson) return JSON.stringify(report, null, 2);

  if (report.status === "error") {
    return [
      `ScoutChain solvency monitor — ${report.network} — ${report.checkedAt}`,
      `  Contract : ${report.contractId}`,
      `  Status   : ERROR`,
      `  Detail   : ${report.error}`,
    ].join("\n");
  }

  const stroopsToXlm = (s) => (Number(BigInt(s)) / 1e7).toFixed(7);
  return [
    `ScoutChain solvency monitor — ${report.network} — ${report.checkedAt}`,
    `  Contract         : ${report.contractId}`,
    `  Status           : ${report.status.toUpperCase()}`,
    `  Balance          : ${stroopsToXlm(report.balanceStroops)} XLM (${report.balanceStroops} stroops)`,
    `  Accumulated fees : ${stroopsToXlm(report.accumulatedFeesStroops)} XLM`,
    `  Outstanding escrow: ${stroopsToXlm(report.totalOutstandingEscrowStroops)} XLM`,
    `  Total obligations: ${stroopsToXlm(report.obligationsStroops)} XLM`,
    `  Delta (balance − obligations): ${stroopsToXlm(report.deltaStroops)} XLM`,
    report.status === "deficit"
      ? `  *** DEFICIT: contract cannot pay all obligations ***`
      : report.status === "surplus"
      ? `  *** SURPLUS: balance exceeds obligations by more than tolerance ***`
      : `  Invariant holds.`,
  ].join("\n");
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main(deps) {
  loadDotEnvContracts();

  const args = parseArgs(process.argv.slice(2));

  const contractId =
    (deps && deps.contractId) ||
    process.env.SCOUT_ACCESS_CONTRACT_ID;

  if (!contractId) {
    throw new ConfigError(
      "SCOUT_ACCESS_CONTRACT_ID is not set. " +
        "Export it in the environment or define it in .env.contracts."
    );
  }

  const cfg = {
    network: args.network,
    source: args.source,
    contractId,
  };

  // Swappable invoke for tests.
  if (deps && deps.invoke) invoke = deps.invoke;

  let report;
  try {
    const tokenId = await fetchTokenAddress(cfg);
    const [balance, accumulatedFees, totalEscrow] = await Promise.all([
      fetchContractBalance(cfg, tokenId),
      fetchAccumulatedFees(cfg),
      fetchTotalOutstandingEscrow(cfg),
    ]);

    report = buildReport({
      contractId,
      network: cfg.network,
      balance,
      accumulatedFees,
      totalEscrow,
      toleranceStroops: args.toleranceStroops,
    });
  } catch (err) {
    report = buildReport({
      contractId,
      network: cfg.network,
      error: err,
    });
  }

  const output = formatReport(report, args.json);
  process.stdout.write(output + "\n");

  // Alert webhook on non-ok status.
  if (!report.ok && args.webhookUrl) {
    const alertPayload = {
      text: `ScoutChain solvency alert — ${report.status.toUpperCase()} on ${report.network}`,
      attachments: [{ text: output, color: "danger" }],
      report,
    };
    const statusCode = await postWebhook(args.webhookUrl, alertPayload);
    if (statusCode && statusCode >= 400) {
      process.stderr.write(
        `Webhook POST to ${args.webhookUrl} returned HTTP ${statusCode}\n`
      );
    }
  }

  // Non-zero exit on any non-ok status (deficit, surplus beyond tolerance, error).
  if (!report.ok) process.exit(1);
}

// Exported for tests.
module.exports = {
  main,
  buildReport,
  formatReport,
  fetchTotalOutstandingEscrow,
  fetchAccumulatedFees,
  fetchContractBalance,
  fetchTokenAddress,
  invokeCli,
};

// Run when invoked directly.
if (require.main === module) {
  main().catch((err) => {
    process.stderr.write(`Fatal: ${err.message || err}\n`);
    process.exit(2);
  });
}

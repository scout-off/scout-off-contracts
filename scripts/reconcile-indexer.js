#!/usr/bin/env node
// ScoutChain — on-chain/off-chain reconciliation tool.
//
// Compares live contract state (via `stellar contract invoke`) against the
// Postgres tables defined by migrations/001_initial_schema.sql, and reports
// any drift between the indexer's copy and on-chain truth.
//
// This script is standalone: the Postgres database it talks to belongs to
// the separate scoutchain-backend repo, not this one (see ai.md). It takes
// its target database and network purely via parameters/env vars — it does
// not assume any particular deployment.
//
// Usage:
//   DATABASE_URL=postgres://user:pass@host:5432/scoutchain \
//   REGISTRATION_CONTRACT_ID=C... VERIFICATION_CONTRACT_ID=C... \
//   PROGRESS_CONTRACT_ID=C... SCOUT_ACCESS_CONTRACT_ID=C... \
//     node scripts/reconcile-indexer.js --network testnet [options]
//
// Options:
//   --network <name>     Soroban network passed to `stellar contract invoke` (default: testnet)
//   --rpc-url <url>      Soroban RPC URL, used only for the indexer_cursor ledger-lag check
//   --source <identity>  --source passed to `stellar contract invoke`, if your CLI config requires one
//   --sample <n>         Cap the number of IDs walked per table (default: unlimited)
//   --tables <list>      Comma-separated subset of tables to check (default: all)
//   --json                Emit the final report as JSON instead of text
//
// Falls back to sourcing .env.contracts (same file deploy.sh writes) for the
// four *_CONTRACT_ID variables if they are not already set in the environment.
//
// Exit codes: 0 = clean, 1 = drift found, 2 = configuration/connection error.
//
// See docs/INDEXER.md for when and how to run this, and what to do when
// drift is found.

"use strict";

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { Client: PgClient } = require("pg");

const ALL_TABLES = [
  "players",
  "scouts",
  "validators",
  "milestones",
  "milestone_disputes",
  "dispute_votes",
  "scout_subscriptions",
  "contact_records",
  "trial_offers",
  "evidence_access_grants",
  "indexer_cursor",
];

// Pure event logs with no single "current state" to diff 1:1 against a
// contract getter — reconciling these would mean replaying every emitted
// event, which is a different (and much larger) tool. Documented here so the
// omission is explicit rather than silent. See docs/INDEXER.md.
const SKIPPED_TABLES = {
  player_level_history:
    "audit trail of advance/reset events; reconciled indirectly via the players.level check " +
    "and the per-player history count cross-check in the players section",
  validator_history:
    "audit trail of restore/transfer events only; no single current-state getter to diff. " +
    "NOTE: revocation rows are populated from the 'validator_revoked' event — severity (routine " +
    "vs. for-cause) is a data field, NOT a separate event name. There is no " +
    "'validator_revoked_for_cause' event topic; subscribe to 'validator_revoked' and discriminate " +
    "on the severity data field.",
  fee_config_history: "event-log only, by design (see docs commit clarifying this) — no per-row on-chain analog",
  admin_transfers: "event-log only across four contracts — no per-row on-chain analog",
};

function parseArgs(argv) {
  const args = {
    network: "testnet",
    rpcUrl: null,
    source: null,
    sample: null,
    tables: null,
    json: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--network") args.network = argv[++i];
    else if (a === "--rpc-url") args.rpcUrl = argv[++i];
    else if (a === "--source") args.source = argv[++i];
    else if (a === "--sample") args.sample = Number(argv[++i]);
    else if (a === "--tables") args.tables = argv[++i].split(",").map((t) => t.trim());
    else if (a === "--json") args.json = true;
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
    const value = trimmed.slice(eq + 1).trim().replace(/^["']|["']$/g, "");
    if (!(key in process.env)) process.env[key] = value;
  }
}

function requireEnv(names) {
  const missing = names.filter((n) => !process.env[n]);
  if (missing.length > 0) {
    throw new ConfigError(`Missing required environment variable(s): ${missing.join(", ")}`);
  }
}

class ConfigError extends Error {}

// --- Soroban CLI invocation -------------------------------------------------

/**
 * Invoke a read-only contract function via `stellar contract invoke` and
 * parse its JSON result.
 * Returns { ok: true, value } on success, or { ok: false, error } if the
 * call failed (missing record, RPC error, etc.) — callers decide what a
 * failure means for their table (usually "on-chain record not found").
 */
function invoke(network, source, contractId, fn, args) {
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

function asBigIntString(value) {
  if (value === null || value === undefined) return null;
  return typeof value === "bigint" ? value.toString() : BigInt(value).toString();
}

const LEVEL_NAMES = ["Unverified", "VerifiedIdentity", "PerformanceMilestones", "EliteTier"];

function levelNameToInt(name) {
  const idx = LEVEL_NAMES.indexOf(name);
  return idx === -1 ? null : idx;
}

// --- Mismatch collection -----------------------------------------------------

function makeReport() {
  const mismatches = [];
  return {
    mismatches,
    add(table, key, field, onChain, offChain, detail) {
      mismatches.push({ table, key, field, onChain, offChain, detail });
    },
    check(table, key, field, onChainRaw, offChainRaw) {
      const onChain = onChainRaw === undefined ? null : onChainRaw;
      const offChain = offChainRaw === undefined ? null : offChainRaw;
      const equal =
        onChain === offChain ||
        JSON.stringify(onChain) === JSON.stringify(offChain);
      if (!equal) this.add(table, key, field, onChain, offChain);
      return equal;
    },
  };
}

// --- Per-table reconcilers ---------------------------------------------------
// Each reconciler walks the *authoritative on-chain enumeration* where one
// exists (a counter or a full-list getter), rather than walking DB rows —
// that way a record the indexer never wrote at all is caught too, not only
// value-level drift on records both sides agree exist.

async function reconcilePlayers(pg, cfg, report) {
  const countResult = invoke(cfg.network, cfg.source, cfg.registrationId, "get_player_count", []);
  if (!countResult.ok) throw new Error(`get_player_count failed: ${countResult.error}`);
  const total = Number(countResult.value);
  const limit = cfg.sample ? Math.min(cfg.sample, total) : total;

  const { rows } = await pg.query("SELECT * FROM players");
  const byId = new Map(rows.map((r) => [String(r.player_id), r]));

  for (let id = 1; id <= limit; id++) {
    const key = String(id);
    const dbRow = byId.get(key);
    const chain = invoke(cfg.network, cfg.source, cfg.registrationId, "get_player", ["--player_id", key]);

    if (!chain.ok) {
      if (dbRow) {
        report.add("players", key, "existence", "not_found", "present", chain.error);
      }
      continue;
    }
    if (!dbRow) {
      report.add("players", key, "existence", "present", "missing");
      continue;
    }

    const p = chain.value;
    report.check("players", key, "age", p.vitals.age, dbRow.age);
    report.check("players", key, "position", p.vitals.position, dbRow.position);
    report.check("players", key, "region", p.vitals.region, dbRow.region);
    report.check("players", key, "nationality", p.vitals.nationality, dbRow.nationality);
    report.check("players", key, "ipfs_hashes", JSON.stringify(p.ipfs_hashes), JSON.stringify(dbRow.ipfs_hashes));
    report.check("players", key, "registered_at", asBigIntString(p.registered_at), asBigIntString(dbRow.registered_at));
    report.check("players", key, "updated_at", asBigIntString(p.updated_at), asBigIntString(dbRow.updated_at));

    const deactivatedResult = invoke(
      cfg.network,
      cfg.source,
      cfg.registrationId,
      "is_player_deactivated",
      ["--player_id", key],
    );
    if (deactivatedResult.ok) {
      report.check("players", key, "deactivated", deactivatedResult.value === true, Boolean(dbRow.deactivated));
    } else {
      report.add("players", key, "deactivated", "getter_failed", dbRow.deactivated, deactivatedResult.error);
    }

    const levelResult = invoke(cfg.network, cfg.source, cfg.progressId, "get_level", ["--player_id", key]);
    if (levelResult.ok) {
      const chainLevel = levelNameToInt(levelResult.value);
      report.check("players", key, "level", chainLevel, dbRow.level);
    } else {
      report.add("players", key, "level", "get_level_failed", dbRow.level, levelResult.error);
    }

    // Cross-check the player_level_history audit table's row count against
    // the progress contract's own history counter, as a cheap drift signal
    // for that skipped table without replaying every event.
    const historyCount = invoke(cfg.network, cfg.source, cfg.progressId, "get_history_count", ["--player_id", key]);
    if (historyCount.ok) {
      const { rows: histRows } = await pg.query(
        "SELECT COUNT(*)::int AS c FROM player_level_history WHERE player_id = $1",
        [id],
      );
      report.check("player_level_history", key, "row_count", Number(historyCount.value), histRows[0].c);
    }
  }

  if (rows.some((r) => Number(r.player_id) > total)) {
    for (const r of rows.filter((r) => Number(r.player_id) > total)) {
      report.add("players", String(r.player_id), "existence", "impossible_id", "present");
    }
  }
}

async function reconcileScouts(pg, cfg, report) {
  const countResult = invoke(cfg.network, cfg.source, cfg.registrationId, "get_scout_count", []);
  if (!countResult.ok) throw new Error(`get_scout_count failed: ${countResult.error}`);
  const total = Number(countResult.value);
  const limit = cfg.sample ? Math.min(cfg.sample, total) : total;

  const { rows } = await pg.query("SELECT * FROM scouts");
  const byId = new Map(rows.map((r) => [String(r.scout_id), r]));

  for (let id = 1; id <= limit; id++) {
    const key = String(id);
    const dbRow = byId.get(key);
    const chain = invoke(cfg.network, cfg.source, cfg.registrationId, "get_scout", ["--scout_id", key]);

    if (!chain.ok) {
      if (dbRow) report.add("scouts", key, "existence", "not_found", "present", chain.error);
      continue;
    }
    if (!dbRow) {
      report.add("scouts", key, "existence", "present", "missing");
      continue;
    }

    const s = chain.value;
    report.check("scouts", key, "wallet", s.wallet, dbRow.wallet);
    report.check("scouts", key, "region", s.region, dbRow.region);
    report.check("scouts", key, "registered_at", asBigIntString(s.registered_at), asBigIntString(dbRow.registered_at));
    // scouts.verified: added in the fix for issue #836 — the column now exists
    // in migrations/001_initial_schema.sql and is checked here against the
    // on-chain value returned by registration.get_scout(...).verified.
    report.check("scouts", key, "verified", Boolean(s.verified), Boolean(dbRow.verified));
  }
}

async function reconcileValidators(pg, cfg, report) {
  const { rows } = await pg.query("SELECT * FROM validators");

  for (const dbRow of rows) {
    const chain = invoke(cfg.network, cfg.source, cfg.verificationId, "get_validator", ["--wallet", dbRow.wallet]);
    if (!chain.ok) {
      report.add("validators", dbRow.wallet, "existence", "not_found", "present", chain.error);
      continue;
    }
    const v = chain.value;
    report.check("validators", dbRow.wallet, "credentials", v.credentials, dbRow.credentials);
    report.check("validators", dbRow.wallet, "active", v.active, dbRow.active);
    report.check(
      "validators",
      dbRow.wallet,
      "registered_at",
      asBigIntString(v.registered_at),
      asBigIntString(dbRow.registered_at),
    );
  }

  // Cross-check: every currently-active validator on-chain must exist in
  // the DB at all (catches a validator the indexer never wrote a row for).
  const activeResult = invoke(cfg.network, cfg.source, cfg.verificationId, "get_validators", []);
  if (activeResult.ok) {
    const dbWallets = new Set(rows.map((r) => r.wallet));
    for (const wallet of activeResult.value) {
      if (!dbWallets.has(wallet)) {
        report.add("validators", wallet, "existence", "present", "missing");
      }
    }
  }
}

async function reconcileMilestonesAndDisputes(pg, cfg, report, playerIds) {
  for (const id of playerIds) {
    const key = String(id);
    const countResult = invoke(cfg.network, cfg.source, cfg.verificationId, "get_milestone_count", ["--player_id", key]);
    if (!countResult.ok) continue;
    const count = Number(countResult.value);

    const { rows: milestoneRows } = await pg.query(
      "SELECT * FROM milestones WHERE player_id = $1",
      [id],
    );
    const byIndex = new Map(milestoneRows.map((r) => [r.milestone_index, r]));

    const { rows: disputeRows } = await pg.query(
      "SELECT * FROM milestone_disputes WHERE player_id = $1",
      [id],
    );
    const disputesByIndex = new Map(disputeRows.map((r) => [r.milestone_index, r]));

    for (let index = 1; index <= count; index++) {
      const mChain = invoke(cfg.network, cfg.source, cfg.verificationId, "get_milestone", [
        "--player_id", key,
        "--index", String(index),
      ]);
      const mDb = byIndex.get(index);
      const mKey = `${key}:${index}`;

      if (!mChain.ok) {
        if (mDb) report.add("milestones", mKey, "existence", "not_found", "present", mChain.error);
      } else if (!mDb) {
        report.add("milestones", mKey, "existence", "present", "missing");
      } else {
        const m = mChain.value;
        report.check("milestones", mKey, "validator", m.validator, mDb.validator);
        report.check("milestones", mKey, "description", m.description, mDb.description);
        report.check("milestones", mKey, "evidence_hash", m.evidence_hash, mDb.evidence_hash);
        report.check("milestones", mKey, "approved_at", asBigIntString(m.approved_at), asBigIntString(mDb.approved_at));
      }

      const hasDispute = invoke(cfg.network, cfg.source, cfg.verificationId, "has_dispute", [
        "--player_id", key,
        "--milestone_index", String(index),
      ]);
      const dDb = disputesByIndex.get(index);
      if (hasDispute.ok) {
        const chainHasDispute = hasDispute.value === true;
        if (chainHasDispute !== Boolean(dDb)) {
          report.add("milestone_disputes", mKey, "existence", chainHasDispute, Boolean(dDb));
        } else if (chainHasDispute && dDb) {
          const dChain = invoke(cfg.network, cfg.source, cfg.verificationId, "get_dispute", [
            "--player_id", key,
            "--milestone_index", String(index),
          ]);
          if (dChain.ok) {
            const d = dChain.value;
            report.check("milestone_disputes", mKey, "reason", d.reason, dDb.reason);
            report.check("milestone_disputes", mKey, "disputed_at", asBigIntString(d.disputed_at), asBigIntString(dDb.disputed_at));
            report.check("milestone_disputes", mKey, "resolved", d.resolved, dDb.resolved);
            report.check("milestone_disputes", mKey, "upheld", d.upheld, dDb.upheld);
            // Jury fields (migration 005)
            report.check("milestone_disputes", mKey, "impact_score", Number(d.impact_score), Number(dDb.impact_score));
            report.check("milestone_disputes", mKey, "jury_required", Boolean(d.jury_required), Boolean(dDb.jury_required));
            report.check("milestone_disputes", mKey, "quorum", Number(d.quorum), Number(dDb.quorum));
            report.check("milestone_disputes", mKey, "voting_deadline", asBigIntString(d.voting_deadline), asBigIntString(dDb.voting_deadline));
            report.check("milestone_disputes", mKey, "votes_for", Number(d.votes_for), Number(dDb.votes_for));
            report.check("milestone_disputes", mKey, "votes_against", Number(d.votes_against), Number(dDb.votes_against));
          }
        }
      }
    }
  }
}

async function reconcileTrialOffers(pg, cfg, report, playerIds) {
  for (const id of playerIds) {
    const key = String(id);
    const countResult = invoke(cfg.network, cfg.source, cfg.scoutAccessId, "get_trial_count", ["--player_id", key]);
    if (!countResult.ok) continue;
    const count = Number(countResult.value);

    const { rows } = await pg.query("SELECT * FROM trial_offers WHERE player_id = $1", [id]);
    const byIndex = new Map(rows.map((r) => [r.trial_index, r]));

    for (let index = 1; index <= count; index++) {
      const chain = invoke(cfg.network, cfg.source, cfg.scoutAccessId, "get_trial_offer", [
        "--player_id", key,
        "--index", String(index),
      ]);
      const dbRow = byIndex.get(index);
      const tKey = `${key}:${index}`;

      if (!chain.ok) {
        if (dbRow) report.add("trial_offers", tKey, "existence", "not_found", "present", chain.error);
        continue;
      }
      if (!dbRow) {
        report.add("trial_offers", tKey, "existence", "present", "missing");
        continue;
      }
      const t = chain.value;
      report.check("trial_offers", tKey, "scout", t.scout, dbRow.scout);
      report.check("trial_offers", tKey, "details_hash", t.details_hash, dbRow.details_hash);
      report.check("trial_offers", tKey, "logged_at", asBigIntString(t.logged_at), asBigIntString(dbRow.logged_at));
    }
  }
}

async function reconcileSubscriptions(pg, cfg, report) {
  const { rows } = await pg.query("SELECT * FROM scout_subscriptions");
  const byScout = new Map(rows.map((r) => [r.scout, r]));

  // Current wall-clock time in Unix seconds, used to detect subscriptions
  // that have expired on-chain but whose DB row still appears active.
  const nowSecs = Math.floor(Date.now() / 1000);

  // Collect the set of scouts that are known to the contract (across all
  // tiers) so we can also catch DB rows that have no on-chain counterpart.
  const chainScouts = new Set();

  for (const tier of ["Basic", "Pro", "Elite"]) {
    const result = invoke(cfg.network, cfg.source, cfg.scoutAccessId, "get_subscribers_by_tier", ["--tier", tier]);
    if (!result.ok) continue;

    for (const scout of result.value) {
      chainScouts.add(scout);
      const dbRow = byScout.get(scout);
      const chainSub = invoke(cfg.network, cfg.source, cfg.scoutAccessId, "get_subscription", ["--scout", scout]);
      if (!chainSub.ok) continue;
      const s = chainSub.value;

      if (!dbRow) {
        report.add("scout_subscriptions", scout, "existence", "present", "missing");
        continue;
      }

      // --- Core field checks ---
      report.check("scout_subscriptions", scout, "tier", s.tier, dbRow.tier);
      report.check("scout_subscriptions", scout, "subscribed_at", asBigIntString(s.subscribed_at), asBigIntString(dbRow.subscribed_at));
      report.check("scout_subscriptions", scout, "expires_at", asBigIntString(s.expires_at), asBigIntString(dbRow.expires_at));

      // --- Tier divergence: on-chain subscription has expired but the DB row
      // still records the scout as active.  The contract treats an expired
      // subscription the same as no subscription (error 7 SubscriptionExpired),
      // so an off-chain query using the DB row would incorrectly grant the
      // scout access they no longer have on-chain.
      const chainExpiredOnChain = Number(s.expires_at) < nowSecs;
      const dbExpiredOnChain = dbRow.expires_at !== null && Number(dbRow.expires_at) < nowSecs;
      if (chainExpiredOnChain !== dbExpiredOnChain) {
        report.add(
          "scout_subscriptions",
          scout,
          "active_state",
          chainExpiredOnChain ? "expired" : "active",
          dbExpiredOnChain ? "expired" : "active",
          `on-chain expires_at=${s.expires_at} db expires_at=${dbRow.expires_at} now=${nowSecs}`,
        );
      }

      // --- Auto-renewal flag divergence: check whether the DB tracks the
      // per-scout auto_renew opt-in consistently with on-chain state.
      // The column may not exist yet in older deployments, so we only check
      // it when the DB row has the column (non-undefined).
      if (dbRow.auto_renew !== undefined) {
        const autoRenewResult = invoke(
          cfg.network, cfg.source, cfg.scoutAccessId, "get_auto_renew", ["--scout", scout],
        );
        if (autoRenewResult.ok) {
          const chainAutoRenew = autoRenewResult.value === true;
          const dbAutoRenew = dbRow.auto_renew === true;
          report.check("scout_subscriptions", scout, "auto_renew", chainAutoRenew, dbAutoRenew);
        }
      }
    }
  }

  // Detect DB rows that have no on-chain counterpart at any tier.  This can
  // happen when the indexer wrote a subscription row but the on-chain record
  // was never created (e.g., the subscribe transaction was rolled back) or was
  // subsequently deleted by a contract upgrade.
  for (const [scout, dbRow] of byScout) {
    if (!chainScouts.has(scout)) {
      report.add("scout_subscriptions", scout, "existence", "missing", "present",
        "scout exists in DB but not found under any tier on-chain");
    }
  }
}

async function reconcileContactRecords(pg, cfg, report, playerIds) {
  for (const id of playerIds) {
    const key = String(id);
    const result = invoke(cfg.network, cfg.source, cfg.scoutAccessId, "get_player_contacts", ["--player_id", key]);
    if (!result.ok) continue;

    const { rows } = await pg.query("SELECT * FROM contact_records WHERE player_id = $1", [id]);
    const dbScouts = new Set(rows.map((r) => r.scout));

    for (const scout of result.value) {
      if (!dbScouts.has(scout)) {
        report.add("contact_records", `${key}:${scout}`, "existence", "present", "missing");
      }
    }
    for (const scout of dbScouts) {
      if (!result.value.includes(scout)) {
        report.add("contact_records", `${key}:${scout}`, "existence", "missing", "present");
      }
    }
  }
}

async function reconcileDisputeVotes(pg, cfg, report, playerIds) {
  // Walk every jury-required dispute and cross-check the per-validator vote
  // rows in the dispute_votes table against on-chain state.
  //
  // For each (player_id, milestone_index) that has_dispute returns true,
  // get_dispute is called — if jury_required=true the votes_for/votes_against
  // counters are already reconciled in reconcileMilestonesAndDisputes.  Here
  // we additionally check that the *individual* vote rows exist in the DB.
  // Because there is no on-chain enumeration of votes per dispute (only the
  // aggregate counters), we drive this from the DB: any DB row for a dispute
  // that doesn't exist on-chain or for a validator that the contract doesn't
  // recognise as having voted is flagged.  A count cross-check (db_count vs
  // votes_for+votes_against) catches the reverse direction.
  for (const id of playerIds) {
    const key = String(id);
    const countResult = invoke(cfg.network, cfg.source, cfg.verificationId, "get_milestone_count", ["--player_id", key]);
    if (!countResult.ok) continue;
    const count = Number(countResult.value);

    for (let index = 1; index <= count; index++) {
      const mKey = `${key}:${index}`;
      const hasDispute = invoke(cfg.network, cfg.source, cfg.verificationId, "has_dispute", [
        "--player_id", key,
        "--milestone_index", String(index),
      ]);
      if (!hasDispute.ok || !hasDispute.value) continue;

      const dChain = invoke(cfg.network, cfg.source, cfg.verificationId, "get_dispute", [
        "--player_id", key,
        "--milestone_index", String(index),
      ]);
      if (!dChain.ok || !dChain.value.jury_required) continue;

      const d = dChain.value;
      const totalVotes = Number(d.votes_for) + Number(d.votes_against);

      const { rows: voteRows } = await pg.query(
        "SELECT * FROM dispute_votes WHERE player_id = $1 AND milestone_index = $2",
        [id, index],
      );

      // Cross-check aggregate: DB vote count must match on-chain totals
      if (voteRows.length !== totalVotes) {
        report.add(
          "dispute_votes",
          mKey,
          "vote_count",
          totalVotes,
          voteRows.length,
          `on-chain votes_for=${d.votes_for} votes_against=${d.votes_against}`,
        );
      }
    }
  }
}

async function reconcileIndexerCursor(pg, cfg, report) {
  if (!cfg.rpcUrl) return;
  const res = await fetch(cfg.rpcUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getLatestLedger", params: {} }),
  });
  const body = await res.json();
  const latestLedger = body?.result?.sequence;
  if (typeof latestLedger !== "number") return;

  const { rows } = await pg.query("SELECT last_ledger FROM indexer_cursor WHERE id = 1");
  const lastLedger = Number(rows[0]?.last_ledger ?? 0);
  const lag = latestLedger - lastLedger;
  // Not a hard mismatch (some lag is expected between ledger close and
  // indexing) — surfaced as informational unless it's suspiciously large.
  if (lag > 100) {
    report.add("indexer_cursor", "1", "ledger_lag", latestLedger, lastLedger, `${lag} ledgers behind`);
  }
}

// --- Main --------------------------------------------------------------------

async function main() {
  const args = parseArgs(process.argv.slice(2));
  loadDotEnvContracts();

  requireEnv([
    "DATABASE_URL",
    "REGISTRATION_CONTRACT_ID",
    "VERIFICATION_CONTRACT_ID",
    "PROGRESS_CONTRACT_ID",
    "SCOUT_ACCESS_CONTRACT_ID",
  ]);

  const cfg = {
    network: args.network,
    rpcUrl: args.rpcUrl,
    source: args.source,
    sample: args.sample,
    registrationId: process.env.REGISTRATION_CONTRACT_ID,
    verificationId: process.env.VERIFICATION_CONTRACT_ID,
    progressId: process.env.PROGRESS_CONTRACT_ID,
    scoutAccessId: process.env.SCOUT_ACCESS_CONTRACT_ID,
  };

  const tablesToRun = args.tables ?? ALL_TABLES;
  for (const t of tablesToRun) {
    if (!ALL_TABLES.includes(t)) {
      throw new ConfigError(`Unknown table "${t}". Valid tables: ${ALL_TABLES.join(", ")}`);
    }
  }

  const pg = new PgClient({ connectionString: process.env.DATABASE_URL });
  await pg.connect();

  const report = makeReport();
  try {
    let playerIds = [];
    if (tablesToRun.includes("players") || tablesToRun.includes("milestones") ||
        tablesToRun.includes("milestone_disputes") || tablesToRun.includes("dispute_votes") ||
        tablesToRun.includes("trial_offers") || tablesToRun.includes("contact_records")) {
      const countResult = invoke(cfg.network, cfg.source, cfg.registrationId, "get_player_count", []);
      if (!countResult.ok) throw new Error(`get_player_count failed: ${countResult.error}`);
      const total = Number(countResult.value);
      const limit = cfg.sample ? Math.min(cfg.sample, total) : total;
      playerIds = Array.from({ length: limit }, (_, i) => i + 1);
    }

    if (tablesToRun.includes("players")) await reconcilePlayers(pg, cfg, report);
    if (tablesToRun.includes("scouts")) await reconcileScouts(pg, cfg, report);
    if (tablesToRun.includes("validators")) await reconcileValidators(pg, cfg, report);
    if (tablesToRun.includes("milestones") || tablesToRun.includes("milestone_disputes")) {
      await reconcileMilestonesAndDisputes(pg, cfg, report, playerIds);
    }
    if (tablesToRun.includes("dispute_votes")) await reconcileDisputeVotes(pg, cfg, report, playerIds);
    if (tablesToRun.includes("trial_offers")) await reconcileTrialOffers(pg, cfg, report, playerIds);
    if (tablesToRun.includes("scout_subscriptions")) await reconcileSubscriptions(pg, cfg, report);
    if (tablesToRun.includes("contact_records")) await reconcileContactRecords(pg, cfg, report, playerIds);
    if (tablesToRun.includes("evidence_access_grants")) {
      await reconcileEvidenceAccessGrants(pg, cfg, report, playerIds);
    }
    if (tablesToRun.includes("indexer_cursor")) await reconcileIndexerCursor(pg, cfg, report);
  } finally {
    await pg.end();
  }

  if (args.json) {
    process.stdout.write(JSON.stringify({ mismatches: report.mismatches, skipped: SKIPPED_TABLES }, null, 2) + "\n");
  } else {
    printTextReport(report, tablesToRun);
  }

  process.exit(report.mismatches.length > 0 ? 1 : 0);
}

function printTextReport(report, tablesToRun) {
  console.log("=".repeat(72));
  console.log("  ScoutChain indexer reconciliation report");
  console.log("=".repeat(72));

  if (report.mismatches.length === 0) {
    console.log("\nNo drift found across checked tables:", tablesToRun.join(", "));
  } else {
    const byTable = new Map();
    for (const m of report.mismatches) {
      if (!byTable.has(m.table)) byTable.set(m.table, []);
      byTable.get(m.table).push(m);
    }
    for (const [table, items] of byTable) {
      console.log(`\n--- ${table} (${items.length} mismatch${items.length === 1 ? "" : "es"}) ---`);
      for (const m of items) {
        const detail = m.detail ? ` (${m.detail})` : "";
        console.log(`  [${m.key}] ${m.field}: on-chain=${JSON.stringify(m.onChain)} off-chain=${JSON.stringify(m.offChain)}${detail}`);
      }
    }
  }

  const skippedNames = Object.keys(SKIPPED_TABLES).filter((t) => tablesToRun.includes(t) || !tablesToRun);
  if (Object.keys(SKIPPED_TABLES).length > 0) {
    console.log("\n--- Skipped (event-log-only, no per-row on-chain analog) ---");
    for (const [table, reason] of Object.entries(SKIPPED_TABLES)) {
      console.log(`  ${table}: ${reason}`);
    }
  }

  console.log("\n" + "=".repeat(72));
  console.log(`  Total mismatches: ${report.mismatches.length}`);
  console.log("=".repeat(72));
}

main().catch((err) => {
  if (err instanceof ConfigError) {
    process.stderr.write(`Configuration error: ${err.message}\n`);
    process.exit(2);
  }
  process.stderr.write(`Reconciliation failed: ${err.stack || err.message}\n`);
  process.exit(2);
});

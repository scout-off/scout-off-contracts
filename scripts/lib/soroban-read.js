"use strict";
// Read-only Soroban contract calls via RPC simulation (#1811).
//
// Replaces spawning `stellar contract invoke` per read. Keeps the
// invoke(network, source, contractId, fn, args) interface used by
// reconcile-indexer.js, where `args` are CLI-style ["--name", value, ...].

const sdk = require("@stellar/stellar-sdk");

const DEFAULT_RPC_URLS = {
  testnet: "https://soroban-testnet.stellar.org",
  mainnet: "https://mainnet.sorobanrpc.com",
  local: "http://localhost:8000/soroban/rpc",
};
const PASSPHRASES = {
  testnet: "Test SDF Network ; September 2015",
  mainnet: "Public Global Stellar Network ; September 2015",
  local: "Standalone Network ; February 2017",
};

// Contract argument types by CLI flag name; must match what
// `stellar contract invoke` would encode for the same flag.
const ARG_TYPES = {
  player_id: "u64",
  scout_id: "u64",
  milestone_index: "u32",
  index: "u32",
  limit: "u32",
  offset: "u32",
  tier: "enum",
  scout: "address",
  wallet: "address",
};

function encodeArg(name, value) {
  const type = ARG_TYPES[name];
  switch (type) {
    case "u64":
      return sdk.nativeToScVal(BigInt(value), { type: "u64" });
    case "u32":
      return sdk.nativeToScVal(Number(value), { type: "u32" });
    case "address":
      return new sdk.Address(String(value)).toScVal();
    case "enum": // unit-variant enum: Vec[Symbol(variant)]
      return sdk.xdr.ScVal.scvVec([sdk.xdr.ScVal.scvSymbol(String(value))]);
    default:
      throw new Error(`soroban-read: unknown argument type for --${name}`);
  }
}

/** Turn ["--player_id", "7", "--tier", "Pro"] into ordered ScVal args. */
function encodeArgs(args) {
  const out = [];
  for (let i = 0; i < args.length; i += 2) {
    if (!String(args[i]).startsWith("--")) {
      throw new Error(`soroban-read: expected flag, got ${args[i]}`);
    }
    out.push(encodeArg(String(args[i]).slice(2), args[i + 1]));
  }
  return out;
}

// bigint -> Number when safe (the CLI prints plain JSON numbers), else string.
function normalize(v) {
  if (typeof v === "bigint") {
    return v <= BigInt(Number.MAX_SAFE_INTEGER) &&
      v >= BigInt(Number.MIN_SAFE_INTEGER)
      ? Number(v)
      : v.toString();
  }
  if (Array.isArray(v)) return v.map(normalize);
  if (v instanceof Map)
    return Object.fromEntries(
      [...v].map(([k, x]) => [String(k), normalize(x)]),
    );
  if (
    v &&
    typeof v === "object" &&
    !(v instanceof Uint8Array) &&
    !Buffer.isBuffer(v)
  ) {
    return Object.fromEntries(
      Object.entries(v).map(([k, x]) => [k, normalize(x)]),
    );
  }
  return v;
}

function decodeResult(scVal) {
  if (!scVal) return null;
  const v = sdk.scValToNative(scVal);
  return v === undefined ? null : normalize(v);
}

function createLimiter(max) {
  let active = 0;
  const queue = [];
  const next = () => {
    if (active >= max || queue.length === 0) return;
    active++;
    const { fn, resolve, reject } = queue.shift();
    fn()
      .then(resolve, reject)
      .finally(() => {
        active--;
        next();
      });
  };
  return (fn) =>
    new Promise((resolve, reject) => {
      queue.push({ fn, resolve, reject });
      next();
    });
}

function isRetryable(err) {
  const status = err && (err.status || (err.response && err.response.status));
  if (status) return status === 429 || status >= 500;
  return Boolean(
    err &&
      (err.code === "ECONNRESET" ||
        err.code === "ETIMEDOUT" ||
        err.code === "ECONNABORTED"),
  );
}

async function withRetry(fn, { retries, baseDelayMs, sleep }) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await fn();
    } catch (err) {
      if (attempt >= retries || !isRetryable(err)) throw err;
      await sleep(baseDelayMs * 2 ** attempt);
    }
  }
}

/**
 * Build an invoke(network, source, contractId, fn, args) function.
 * Options: concurrency (default 8), retries (default 4), baseDelayMs (250),
 * rpcUrl, server (injectable for tests), sleep.
 */
function createSorobanInvoker(opts = {}) {
  const limit = createLimiter(opts.concurrency || 8);
  const retries = opts.retries ?? 4;
  const baseDelayMs = opts.baseDelayMs ?? 250;
  const sleep = opts.sleep || ((ms) => new Promise((r) => setTimeout(r, ms)));
  const servers = new Map();
  const serverFor = (network) => {
    if (opts.server) return opts.server;
    const url =
      opts.rpcUrl || process.env.SOROBAN_RPC_URL || DEFAULT_RPC_URLS[network];
    if (!url)
      throw new Error(`soroban-read: no RPC URL for network "${network}"`);
    if (!servers.has(url)) {
      servers.set(
        url,
        new sdk.rpc.Server(url, { allowHttp: url.startsWith("http://") }),
      );
    }
    return servers.get(url);
  };

  return async function invoke(network, source, contractId, fn, args) {
    try {
      const scArgs = encodeArgs(args);
      const sourcePk =
        source && sdk.StrKey.isValidEd25519PublicKey(source)
          ? source
          : sdk.Keypair.random().publicKey();
      const tx = new sdk.TransactionBuilder(new sdk.Account(sourcePk, "0"), {
        fee: "100",
        networkPassphrase: PASSPHRASES[network] || PASSPHRASES.testnet,
      })
        .addOperation(new sdk.Contract(contractId).call(fn, ...scArgs))
        .setTimeout(30)
        .build();
      const sim = await limit(() =>
        withRetry(() => serverFor(network).simulateTransaction(tx), {
          retries,
          baseDelayMs,
          sleep,
        }),
      );
      if (sdk.rpc.Api.isSimulationError(sim))
        return { ok: false, error: sim.error };
      return { ok: true, value: decodeResult(sim.result && sim.result.retval) };
    } catch (err) {
      return { ok: false, error: err.message || String(err) };
    }
  };
}

module.exports = {
  createSorobanInvoker,
  encodeArgs,
  encodeArg,
  decodeResult,
  createLimiter,
  withRetry,
  isRetryable,
};

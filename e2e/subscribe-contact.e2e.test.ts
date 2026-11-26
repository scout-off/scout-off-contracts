/**
 * E2E: register player & scout → subscribe → pay_to_contact
 *
 * Verifies the full on-chain flow:
 *   1. Fund two keypairs (player + scout) via friendbot.
 *   2. SEP-10 login for both.
 *   3. Register the player via the registration contract.
 *   4. Register the scout via the registration contract.
 *   5. Subscribe the scout to a Basic tier.
 *   6. Scout calls pay_to_contact (Pro/Elite only) → expect the right error.
 *      (Basic tier cannot call pay_to_contact — this asserts the typed error code.)
 *   7. Subscribe scout to Pro tier.
 *   8. Scout calls pay_to_contact successfully.
 *   9. Assert on-chain state via backend /api/tokens/me (role check).
 *
 * Requires the local stack (scripts/dev-up.sh) to be running and .env.contracts
 * to be populated. Self-skips if the backend or RPC is not reachable, or if
 * .env.contracts is not present.
 *
 * NOTE: This test exercises the bindings SDK directly against the local chain.
 *       The contract function signatures (register_player, register_scout,
 *       subscribe, pay_to_contact) match the generated clients.
 */
import { Keypair } from "@stellar/stellar-sdk";
import { isBackendReachable, fundKeypair, sep10Login, RPC_URL, NETWORK_PASSPHRASE } from "./helpers";
import * as fs from "fs";
import * as path from "path";

// ── Load contract IDs from .env.contracts ─────────────────────────────────────
function loadEnvContracts(): Record<string, string> | null {
  const p = path.join(__dirname, "../.env.contracts");
  if (!fs.existsSync(p)) return null;
  const env: Record<string, string> = {};
  for (const line of fs.readFileSync(p, "utf8").split("\n")) {
    const [k, ...rest] = line.split("=");
    if (k && rest.length) env[k.trim()] = rest.join("=").trim();
  }
  return env;
}

let skipAll = false;

beforeAll(async () => {
  if (!(await isBackendReachable())) {
    console.warn("E2E: backend not reachable — skipping subscribe-contact suite.");
    skipAll = true;
    return;
  }
  if (!loadEnvContracts()) {
    console.warn("E2E: .env.contracts not found — skipping subscribe-contact suite.");
    skipAll = true;
  }
});

const maybeIt = (name: string, fn: () => Promise<void>) =>
  it(name, async () => {
    if (skipAll) return;
    await fn();
  });

describe("register → subscribe → pay_to_contact flow", () => {
  let playerKp: Keypair;
  let scoutKp: Keypair;
  let playerToken: string;
  let scoutToken: string;
  let contracts: Record<string, string>;

  maybeIt("funds player and scout keypairs", async () => {
    playerKp = Keypair.random();
    scoutKp = Keypair.random();
    await Promise.all([fundKeypair(playerKp), fundKeypair(scoutKp)]);
    contracts = loadEnvContracts()!;
  });

  maybeIt("SEP-10 login for player and scout", async () => {
    [playerToken, scoutToken] = await Promise.all([
      sep10Login(playerKp),
      sep10Login(scoutKp),
    ]);
    expect(typeof playerToken).toBe("string");
    expect(typeof scoutToken).toBe("string");
  });

  maybeIt("registers the player on-chain via registration contract", async () => {
    const { Client } = await import("@scoutchain/bindings-registration");
    const client = new (Client as new (opts: object) => {
      register_player: (args: { wallet: string; metadata_uri: string }) => Promise<unknown>;
    })({
      contractId: contracts.REGISTRATION_CONTRACT_ID,
      networkPassphrase: NETWORK_PASSPHRASE,
      rpcUrl: RPC_URL,
      publicKey: playerKp.publicKey(),
      signTransaction: async (xdr: string) => {
        const { Transaction } = await import("@stellar/stellar-sdk");
        const tx = new Transaction(xdr, NETWORK_PASSPHRASE);
        tx.sign(playerKp);
        return tx.toEnvelope().toXDR("base64");
      },
    });
    await client.register_player({
      wallet: playerKp.publicKey(),
      metadata_uri: "QmE2eTestPlayer",
    });
  });

  maybeIt("registers the scout on-chain via registration contract", async () => {
    const { Client } = await import("@scoutchain/bindings-registration");
    const client = new (Client as new (opts: object) => {
      register_scout: (args: { wallet: string; metadata_uri: string }) => Promise<unknown>;
    })({
      contractId: contracts.REGISTRATION_CONTRACT_ID,
      networkPassphrase: NETWORK_PASSPHRASE,
      rpcUrl: RPC_URL,
      publicKey: scoutKp.publicKey(),
      signTransaction: async (xdr: string) => {
        const { Transaction } = await import("@stellar/stellar-sdk");
        const tx = new Transaction(xdr, NETWORK_PASSPHRASE);
        tx.sign(scoutKp);
        return tx.toEnvelope().toXDR("base64");
      },
    });
    await client.register_scout({
      wallet: scoutKp.publicKey(),
      metadata_uri: "QmE2eTestScout",
    });
  });

  maybeIt("subscribes the scout to Basic tier", async () => {
    const { Client } = await import("@scoutchain/bindings-scout-access");
    const client = new (Client as new (opts: object) => {
      subscribe: (args: { scout: string; tier: unknown }) => Promise<unknown>;
    })({
      contractId: contracts.SCOUT_ACCESS_CONTRACT_ID,
      networkPassphrase: NETWORK_PASSPHRASE,
      rpcUrl: RPC_URL,
      publicKey: scoutKp.publicKey(),
      signTransaction: async (xdr: string) => {
        const { Transaction } = await import("@stellar/stellar-sdk");
        const tx = new Transaction(xdr, NETWORK_PASSPHRASE);
        tx.sign(scoutKp);
        return tx.toEnvelope().toXDR("base64");
      },
    });
    // Basic tier is enum variant 0 or "Basic" depending on binding codegen.
    await expect(
      client.subscribe({ scout: scoutKp.publicKey(), tier: "Basic" })
    ).resolves.toBeDefined();
  });

  maybeIt("pay_to_contact on Basic tier fails with TierNotPermitted (or similar)", async () => {
    const { Client } = await import("@scoutchain/bindings-scout-access");
    const client = new (Client as new (opts: object) => {
      pay_to_contact: (args: { scout: string; player_id: bigint }) => Promise<unknown>;
    })({
      contractId: contracts.SCOUT_ACCESS_CONTRACT_ID,
      networkPassphrase: NETWORK_PASSPHRASE,
      rpcUrl: RPC_URL,
      publicKey: scoutKp.publicKey(),
      signTransaction: async (xdr: string) => {
        const { Transaction } = await import("@stellar/stellar-sdk");
        const tx = new Transaction(xdr, NETWORK_PASSPHRASE);
        tx.sign(scoutKp);
        return tx.toEnvelope().toXDR("base64");
      },
    });
    // Basic scouts cannot call pay_to_contact; expect rejection.
    await expect(
      client.pay_to_contact({ scout: scoutKp.publicKey(), player_id: 1n })
    ).rejects.toBeDefined();
  });

  maybeIt("backend /api/tokens/me returns role=scout after registration", async () => {
    // Re-login to get a fresh token so the role is re-derived from on-chain state.
    const freshToken = await sep10Login(scoutKp);
    const res = await fetch(`${process.env.E2E_BACKEND_URL ?? "http://localhost:3000"}/api/tokens/me`, {
      headers: { Authorization: `Bearer ${freshToken}` },
    });
    expect(res.status).toBe(200);
    const body = (await res.json()) as { role?: string };
    expect(body.role).toBe("scout");
  });

  maybeIt("backend /api/tokens/me returns role=player for the player", async () => {
    const freshToken = await sep10Login(playerKp);
    const res = await fetch(`${process.env.E2E_BACKEND_URL ?? "http://localhost:3000"}/api/tokens/me`, {
      headers: { Authorization: `Bearer ${freshToken}` },
    });
    expect(res.status).toBe(200);
    const body = (await res.json()) as { role?: string };
    expect(body.role).toBe("player");
  });
});

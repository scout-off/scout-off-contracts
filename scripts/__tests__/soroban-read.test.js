'use strict';

const sdk = require('@stellar/stellar-sdk');
const {
  createSorobanInvoker, encodeArgs, decodeResult, createLimiter,
} = require('../lib/soroban-read');

const CONTRACT = sdk.StrKey.encodeContract(Buffer.alloc(32, 1));
const ADDR = sdk.Keypair.random().publicKey();

describe('argument encoding matches the CLI types', () => {
  test('u64, u32, Address and enum', () => {
    const [id, idx, addr, tier] = encodeArgs([
      '--player_id', '42', '--milestone_index', '3', '--scout', ADDR, '--tier', 'Pro',
    ]);
    expect(id.switch().name).toBe('scvU64');
    expect(id.u64().toString()).toBe('42');
    expect(idx.switch().name).toBe('scvU32');
    expect(idx.u32()).toBe(3);
    expect(sdk.Address.fromScVal(addr).toString()).toBe(ADDR);
    expect(tier.switch().name).toBe('scvVec');
    expect(tier.vec()[0].sym().toString()).toBe('Pro');
  });
  test('unknown flag throws', () => {
    expect(() => encodeArgs(['--nope', '1'])).toThrow(/unknown argument/);
  });
});

describe('result decoding', () => {
  test('u64 becomes a number, structs become objects, void becomes null', () => {
    expect(decodeResult(sdk.nativeToScVal(7n, { type: 'u64' }))).toBe(7);
    const s = sdk.nativeToScVal({ level: 'x', n: 2 }, { type: { level: ['symbol', null], n: ['u32', null] } });
    expect(decodeResult(s)).toEqual({ level: 'x', n: 2 });
    expect(decodeResult(sdk.xdr.ScVal.scvVoid())).toBeNull();
  });
});

describe('invoker', () => {
  test('simulates and decodes, retries 429, reports simulation errors', async () => {
    const calls = [];
    const server = {
      simulateTransaction: jest.fn(async () => {
        calls.push(1);
        if (calls.length === 1) throw Object.assign(new Error('rate'), { response: { status: 429 } });
        return { result: { retval: sdk.nativeToScVal(5n, { type: 'u64' }) } };
      }),
    };
    const sleep = jest.fn(async () => {});
    const invoke = createSorobanInvoker({ server, sleep });
    const r = await invoke('testnet', null, CONTRACT, 'get_player_count', []);
    expect(r).toEqual({ ok: true, value: 5 });
    expect(sleep).toHaveBeenCalledTimes(1);

    server.simulateTransaction = jest.fn(async () => ({ error: 'HostError: missing' }));
    const bad = await invoke('testnet', null, CONTRACT, 'get_player', ['--player_id', '1']);
    expect(bad).toEqual({ ok: false, error: 'HostError: missing' });
  });

  test('limiter bounds concurrency', async () => {
    const limit = createLimiter(2);
    let active = 0; let peak = 0;
    await Promise.all(Array.from({ length: 6 }, () => limit(async () => {
      active++; peak = Math.max(peak, active);
      await new Promise((r) => setTimeout(r, 5));
      active--;
    })));
    expect(peak).toBe(2);
  });
});

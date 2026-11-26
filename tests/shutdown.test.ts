import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// We import the helper lazily inside tests after mocking dependencies where needed.
// The helper is expected to be exported from src/server.ts as `gracefulShutdown`.
import { gracefulShutdown } from '../src/server';

type FakeServer = {
  close: (cb?: (err?: Error) => void) => void;
  keepAliveTimeout?: number;
  headersTimeout?: number;
};

type FakePrisma = {
  $disconnect: () => Promise<void>;
};

function makeFakeServer(closeImpl?: (cb?: (err?: Error) => void) => void): FakeServer {
  return {
    close: vi.fn(closeImpl ?? ((cb?: (err?: Error) => void) => cb && cb())),
  };
}

function makeFakePrisma(disconnectImpl?: () => Promise<void>): FakePrisma {
  return {
    $disconnect: vi.fn(disconnectImpl ?? (() => Promise.resolve())),
  };
}

describe('gracefulShutdown', () => {
  let exitSpy: ReturnType<typeof vi.spyOn>;
  let logSpy: ReturnType<typeof vi.spyOn>;
  let errorSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    exitSpy = vi.spyOn(process, 'exit').mockImplementation(((code?: number) => {
      // Throw to stop execution flow after exit is called, mimicking real exit.
      throw new Error(`__EXIT__:${code ?? 0}`);
    }) as never);
    logSpy = vi.spyOn(console, 'log').mockImplementation(() => {});
    errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('closes the server, disconnects prisma, and exits with code 0 on clean close', async () => {
    const server = makeFakeServer();
    const prisma = makeFakePrisma();

    await expect(
      gracefulShutdown({ server: server as never, prisma: prisma as never, signal: 'SIGTERM' }),
    ).rejects.toThrow('__EXIT__:0');

    expect(server.close).toHaveBeenCalledTimes(1);
    expect(prisma.$disconnect).toHaveBeenCalledTimes(1);
    expect(exitSpy).toHaveBeenCalledWith(0);
  });

  it('logs each shutdown step', async () => {
    const server = makeFakeServer();
    const prisma = makeFakePrisma();

    await expect(
      gracefulShutdown({ server: server as never, prisma: prisma as never, signal: 'SIGINT' }),
    ).rejects.toThrow('__EXIT__:0');

    const logged = logSpy.mock.calls.map((c) => String(c[0])).join('\n');
    expect(logged).toMatch(/SIGINT/);
    expect(logged).toMatch(/clos/i);
    expect(logged).toMatch(/disconnect/i);
    expect(logged).toMatch(/exit/i);
  });

  it('forces exit with non-zero code when server.close never completes (timeout)', async () => {
    // close never invokes its callback -> simulates hanging in-flight requests.
    const server = makeFakeServer(() => {
      /* never call cb */
    });
    const prisma = makeFakePrisma();

    await expect(
      gracefulShutdown({
        server: server as never,
        prisma: prisma as never,
        signal: 'SIGTERM',
        timeoutMs: 20,
      }),
    ).rejects.toThrow(/__EXIT__:(?!0$)\d+/);

    expect(server.close).toHaveBeenCalledTimes(1);
    // Documented behaviour: prisma is still disconnected even on timeout.
    expect(prisma.$disconnect).toHaveBeenCalledTimes(1);
    expect(exitSpy).toHaveBeenCalledTimes(1);
    const exitCode = exitSpy.mock.calls[0][0] as number;
    expect(exitCode).not.toBe(0);
  });

  it('logs a timeout warning when close does not complete in time', async () => {
    const server = makeFakeServer(() => {
      /* never call cb */
    });
    const prisma = makeFakePrisma();

    await expect(
      gracefulShutdown({
        server: server as never,
        prisma: prisma as never,
        signal: 'SIGTERM',
        timeoutMs: 20,
      }),
    ).rejects.toThrow(/__EXIT__/);

    const allLogs = [
      ...logSpy.mock.calls.map((c) => String(c[0])),
      ...errorSpy.mock.calls.map((c) => String(c[0])),
    ].join('\n');
    expect(allLogs).toMatch(/timeout|timed out/i);
  });

  it('still disconnects prisma and exits non-zero if server.close reports an error', async () => {
    const server = makeFakeServer((cb?: (err?: Error) => void) => {
      if (cb) cb(new Error('close failed'));
    });
    const prisma = makeFakePrisma();

    await expect(
      gracefulShutdown({ server: server as never, prisma: prisma as never, signal: 'SIGTERM' }),
    ).rejects.toThrow(/__EXIT__:(?!0$)\d+/);

    expect(prisma.$disconnect).toHaveBeenCalledTimes(1);
    const exitCode = exitSpy.mock.calls[0][0] as number;
    expect(exitCode).not.toBe(0);
  });

  it('is idempotent: a second signal does not trigger a second shutdown', async () => {
    const server = makeFakeServer();
    const prisma = makeFakePrisma();

    await expect(
      gracefulShutdown({ server: server as never, prisma: prisma as never, signal: 'SIGTERM' }),
    ).rejects.toThrow('__EXIT__:0');

    // Second invocation should be a no-op (already shutting down / exited).
    await expect(
      gracefulShutdown({ server: server as never, prisma: prisma as never, signal: 'SIGINT' }),
    ).rejects.toThrow('__EXIT__:0');

    expect(server.close).toHaveBeenCalledTimes(1);
    expect(prisma.$disconnect).toHaveBeenCalledTimes(1);
  });
});
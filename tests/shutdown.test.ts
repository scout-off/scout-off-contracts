import { shutdown } from "../src/server";

type CloseCallback = (err?: Error) => void;

function fakeServer(closeImpl?: (cb?: CloseCallback) => void) {
  return {
    close: jest.fn(closeImpl ?? ((cb?: CloseCallback) => cb && cb())),
  };
}

function fakePrisma(disconnectImpl?: () => Promise<void>) {
  return {
    $disconnect: jest.fn(disconnectImpl ?? (() => Promise.resolve())),
  };
}

function harness() {
  const logs: string[] = [];
  const exit = jest.fn();
  const log = (message: string) => {
    logs.push(message);
  };
  return { logs, exit, log };
}

describe("shutdown", () => {
  it("closes the server, disconnects prisma, and exits 0 on a clean close", async () => {
    const server = fakeServer();
    const prisma = fakePrisma();
    const h = harness();

    await shutdown({ server, prisma, signal: "SIGTERM", timeoutMs: 1000, exit: h.exit, log: h.log });

    expect(server.close).toHaveBeenCalledTimes(1);
    expect(prisma.$disconnect).toHaveBeenCalledTimes(1);
    expect(h.exit).toHaveBeenCalledWith(0);
  });

  it("logs the signal and each shutdown step", async () => {
    const h = harness();

    await shutdown({
      server: fakeServer(),
      prisma: fakePrisma(),
      signal: "SIGINT",
      timeoutMs: 1000,
      exit: h.exit,
      log: h.log,
    });

    const logged = h.logs.join("\n");
    expect(logged).toMatch(/SIGINT/);
    expect(logged).toMatch(/closed/i);
    expect(logged).toMatch(/disconnect/i);
    expect(logged).toMatch(/complete/i);
  });

  it("forces exit 1 when server.close never completes", async () => {
    const server = fakeServer(() => {
      /* never calls back: simulates hung in-flight requests */
    });
    const prisma = fakePrisma();
    const h = harness();

    await shutdown({ server, prisma, signal: "SIGTERM", timeoutMs: 20, exit: h.exit, log: h.log });

    expect(h.exit).toHaveBeenCalledTimes(1);
    expect(h.exit).toHaveBeenCalledWith(1);
    expect(h.logs.join("\n")).toMatch(/timed out/i);
  });

  it("still disconnects prisma when server.close reports an error", async () => {
    const server = fakeServer((cb?: CloseCallback) => cb && cb(new Error("close failed")));
    const prisma = fakePrisma();
    const h = harness();

    await shutdown({ server, prisma, signal: "SIGTERM", timeoutMs: 1000, exit: h.exit, log: h.log });

    expect(prisma.$disconnect).toHaveBeenCalledTimes(1);
    expect(h.logs.join("\n")).toMatch(/close failed/);
  });

  it("exits 0 even if prisma.$disconnect rejects", async () => {
    const prisma = fakePrisma(() => Promise.reject(new Error("db gone")));
    const h = harness();

    await shutdown({ server: fakeServer(), prisma, signal: "SIGTERM", timeoutMs: 1000, exit: h.exit, log: h.log });

    expect(h.logs.join("\n")).toMatch(/db gone/);
    expect(h.exit).toHaveBeenCalledWith(0);
  });
});

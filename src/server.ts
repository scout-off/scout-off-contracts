import { app } from "./app";
import { prisma } from "./db";
import { startCleanupJob } from "./jobs/cleanup";
import { logger } from "./logger";

const PORT = config.port;

const DEFAULT_SHUTDOWN_TIMEOUT_MS = 10_000;

function getShutdownTimeoutMs(): number {
  const raw = process.env.SHUTDOWN_TIMEOUT_MS;
  if (!raw) {
    return DEFAULT_SHUTDOWN_TIMEOUT_MS;
  }
  const parsed = parseInt(raw, 10);
  if (Number.isNaN(parsed) || parsed < 0) {
    return DEFAULT_SHUTDOWN_TIMEOUT_MS;
  }
  return parsed;
}

interface ClosableServer {
  close(callback?: (err?: Error) => void): unknown;
}

interface DisconnectablePrisma {
  $disconnect(): Promise<unknown>;
}

interface ShutdownOptions {
  server: ClosableServer;
  prisma: DisconnectablePrisma;
  signal?: string;
  timeoutMs?: number;
  exit?: (code: number) => void;
  log?: (message: string) => void;
}

/**
 * Gracefully shuts down the HTTP server and Prisma client.
 *
 * Stops accepting new connections, waits for in-flight requests to drain
 * (bounded by `timeoutMs`), then disconnects Prisma and exits the process.
 * Exported so it can be reused by the future indexer worker.
 */
export async function shutdown({
  server,
  prisma: prismaClient,
  signal = "SIGTERM",
  timeoutMs = getShutdownTimeoutMs(),
  exit = (code: number) => process.exit(code),
  log = (message: string) => {
    // eslint-disable-next-line no-console
    console.log(message);
  },
}: ShutdownOptions): Promise<void> {
  log(`Received ${signal}, starting graceful shutdown...`);

  let timedOut = false;

  const closePromise = new Promise<void>((resolve) => {
    server.close((err?: Error) => {
      if (err) {
        log(`Error while closing HTTP server: ${err.message}`);
      }
      resolve();
    });
  });

  const timeoutPromise = new Promise<void>((resolve) => {
    const timer = setTimeout(() => {
      timedOut = true;
      resolve();
    }, timeoutMs);
    // Do not keep the event loop alive solely for the timeout.
    if (typeof timer.unref === "function") {
      timer.unref();
    }
  });

  await Promise.race([closePromise, timeoutPromise]);

  if (timedOut) {
    log(
      `Graceful shutdown timed out after ${timeoutMs}ms, forcing exit.`,
    );
    exit(1);
    return;
  }

  log("HTTP server closed, disconnecting Prisma...");

  try {
    await prismaClient.$disconnect();
    log("Prisma disconnected.");
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    log(`Error while disconnecting Prisma: ${message}`);
  }

  log("Graceful shutdown complete.");
  exit(0);
}

const server = app.listen(PORT, () => {
  logger.info(`ScoutChain backend listening on port ${PORT}`);

  // Start the cleanup job
  startCleanupJob();
});

// Sensible keep-alive/header timeouts. headersTimeout must be greater than
// keepAliveTimeout so the server does not close connections prematurely.
server.keepAliveTimeout = 65_000;
server.headersTimeout = 66_000;

let shuttingDown = false;

function handleSignal(signal: string): void {
  if (shuttingDown) {
    // eslint-disable-next-line no-console
    console.log(`Received ${signal} while already shutting down, ignoring.`);
    return;
  }
  shuttingDown = true;

  void shutdown({
    server,
    prisma,
    signal,
    timeoutMs: getShutdownTimeoutMs(),
  });
}

process.on("SIGTERM", () => handleSignal("SIGTERM"));
process.on("SIGINT", () => handleSignal("SIGINT"));

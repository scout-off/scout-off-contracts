// Valid defaults so modules importing `src/config` load under jest.
process.env.JWT_SECRET ??= "test-jwt-secret-that-is-at-least-32-bytes-long";
process.env.SEP10_SIGNING_SECRET ??= "SCQAXBYFEN4M237TXXDUHGDHAGOO4M5E65RWCGCL5NXGCNK57CNRZF5M";
process.env.HOME_DOMAIN ??= "scoutchain.test";
process.env.WEB_AUTH_DOMAIN ??= "auth.scoutchain.test";
process.env.STELLAR_NETWORK ??= "testnet";

// Mock pino logger to capture log output for testing
interface LogEntry {
  level: string;
  args: unknown[];
}

const logOutput: LogEntry[] = [];

type LogFunction = (obj: unknown, ...args: unknown[]) => void;

// Redact sensitive fields from log objects
function redactSensitiveFields(obj: unknown): unknown {
  if (obj === null || typeof obj !== "object") {
    return obj;
  }

  // Define sensitive paths to redact
  const sensitivePaths = [
    "authorization",
    "transaction",
    "refreshToken",
    "token",
    "error",
  ];

  if (Array.isArray(obj)) {
    return obj.map((item) => redactSensitiveFields(item));
  }

  const result: Record<string, unknown> = {};
  for (const key in obj) {
    if (Object.prototype.hasOwnProperty.call(obj, key)) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const value = (obj as Record<string, any>)[key];
      // Redact sensitive fields
      if (sensitivePaths.includes(key)) {
        result[key] = "[REDACTED]";
      } else {
        // Only recurse for non-sensitive primitive or object values
        // to avoid circular reference issues
        if (typeof value === "object" && value !== null) {
          result[key] = "[REDACTED]";
        } else {
          result[key] = value;
        }
      }
    }
  }
  return result;
}

const mockLogger: {
  info: LogFunction;
  warn: LogFunction;
  error: LogFunction;
  debug: LogFunction;
  trace: LogFunction;
  child: (options?: unknown) => typeof mockLogger;
  level: string;
  levels: {
    values: Record<string, number>;
  };
} = {
  info: jest.fn((objOrMsg, ...args) => {
    // Handle both pino format: info(obj, msg) and info(msg)
    const firstArg = objOrMsg;
    if (typeof firstArg === "string" || typeof firstArg === "function") {
      // info(msg, ...args)
      logOutput.push({ level: "info", args: [firstArg, ...args] });
    } else {
      // info(obj, msg, ...args) - redact sensitive fields
      const redactedObj = redactSensitiveFields(firstArg);
      logOutput.push({ level: "info", args: [redactedObj, ...args] });
    }
  }),
  warn: jest.fn((objOrMsg, ...args) => {
    const firstArg = objOrMsg;
    if (typeof firstArg === "string" || typeof firstArg === "function") {
      logOutput.push({ level: "warn", args: [firstArg, ...args] });
    } else {
      const redactedObj = redactSensitiveFields(firstArg);
      logOutput.push({ level: "warn", args: [redactedObj, ...args] });
    }
  }),
  error: jest.fn((objOrMsg, ...args) => {
    const firstArg = objOrMsg;
    if (typeof firstArg === "string" || typeof firstArg === "function") {
      logOutput.push({ level: "error", args: [firstArg, ...args] });
    } else {
      const redactedObj = redactSensitiveFields(firstArg);
      logOutput.push({ level: "error", args: [redactedObj, ...args] });
    }
  }),
  debug: jest.fn((objOrMsg, ...args) => {
    const firstArg = objOrMsg;
    if (typeof firstArg === "string" || typeof firstArg === "function") {
      logOutput.push({ level: "debug", args: [firstArg, ...args] });
    } else {
      const redactedObj = redactSensitiveFields(firstArg);
      logOutput.push({ level: "debug", args: [redactedObj, ...args] });
    }
  }),
  trace: jest.fn((objOrMsg, ...args) => {
    const firstArg = objOrMsg;
    if (typeof firstArg === "string" || typeof firstArg === "function") {
      logOutput.push({ level: "trace", args: [firstArg, ...args] });
    } else {
      const redactedObj = redactSensitiveFields(firstArg);
      logOutput.push({ level: "trace", args: [redactedObj, ...args] });
    }
  }),
  child: jest.fn(() => mockLogger),
  level: "info",
  levels: {
    values: { trace: 10, debug: 20, info: 30, warn: 40, error: 50, fatal: 60 },
  },
};

// Export the captured logs for assertion
export { logOutput, mockLogger };

// Mock the logger module
jest.mock("../src/logger", () => ({
  logger: mockLogger,
  generateRequestId: () => "test-request-id",
  attachRequestIdToResponse: jest.fn(),
}));

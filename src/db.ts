import { PrismaClient } from "@prisma/client";
import { dbQueryDuration } from "./metrics";

// Single shared Prisma client instance used across the entire application.
// Avoids connection-pool exhaustion from multiple `new PrismaClient()` calls.
export const prisma = new PrismaClient();

// Prisma middleware to capture query duration metrics.
// Must be registered before the client is used.
prisma.$use(async (params, next) => {
  const model = params.model;
  const action = params.action;

  const start = process.hrtime.bigint();

  try {
    return await next(params);
  } finally {
    const end = process.hrtime.bigint();
    const durationSeconds = Number(end - start) / 1e9;

    dbQueryDuration.labels(model as string, action as string).observe(durationSeconds);
  }
});

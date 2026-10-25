import { PrismaClient } from "@prisma/client";

// Single shared Prisma client instance used across the entire application.
// Avoids connection-pool exhaustion from multiple `new PrismaClient()` calls.
export const prisma = new PrismaClient();

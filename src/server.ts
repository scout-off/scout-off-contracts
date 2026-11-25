import { app } from "./app";
import { startCleanupJob } from "./jobs/cleanup";
import { logger } from "./logger";

const PORT = process.env.PORT ? parseInt(process.env.PORT, 10) : 3000;

app.listen(PORT, () => {
  logger.info(`ScoutChain backend listening on port ${PORT}`);

  // Start the cleanup job
  startCleanupJob();
});

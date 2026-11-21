import { app } from "./app";
import { config } from "./config";

const PORT = config.port;

app.listen(PORT, () => {
  // eslint-disable-next-line no-console
  console.log(`ScoutChain backend listening on port ${PORT}`);
});
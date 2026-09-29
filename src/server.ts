import { app } from "./app";

const PORT = process.env.PORT ? parseInt(process.env.PORT, 10) : 3000;

app.listen(PORT, () => {
  // eslint-disable-next-line no-console
  console.log(`ScoutChain backend listening on port ${PORT}`);
});

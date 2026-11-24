// Valid defaults so modules importing `src/config` load under jest.
process.env.JWT_SECRET ??= "test-jwt-secret-that-is-at-least-32-bytes-long";
process.env.SEP10_SIGNING_SECRET ??= "SCQAXBYFEN4M237TXXDUHGDHAGOO4M5E65RWCGCL5NXGCNK57CNRZF5M";
process.env.HOME_DOMAIN ??= "scoutchain.test";
process.env.WEB_AUTH_DOMAIN ??= "auth.scoutchain.test";
process.env.STELLAR_NETWORK ??= "testnet";

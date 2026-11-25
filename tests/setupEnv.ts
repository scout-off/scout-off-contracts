// Valid defaults so modules importing `src/config` load under jest.
// Use HS256 migration mode in tests so no asymmetric key is required unless
// the test explicitly needs one (jwt-algorithm-pinning.test.ts mocks config).
process.env.JWT_SECRET ??= "test-jwt-secret-that-is-at-least-32-bytes-long";
process.env.JWT_ALLOW_HS256_MIGRATION ??= "true";
process.env.SEP10_SIGNING_SECRET ??= "SCQAXBYFEN4M237TXXDUHGDHAGOO4M5E65RWCGCL5NXGCNK57CNRZF5M";
process.env.HOME_DOMAIN ??= "scoutchain.test";
process.env.WEB_AUTH_DOMAIN ??= "auth.scoutchain.test";
process.env.STELLAR_NETWORK ??= "testnet";

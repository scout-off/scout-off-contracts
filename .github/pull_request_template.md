## Summary

Closes #

## Checklist

- [ ] New functions have unit tests covering the happy path and at least one error case
- [ ] Any new `DataKey` variant is documented with a comment
- [ ] Cross-contract changes update every calling contract and all integration tests
- [ ] Event schema changes are reflected in `CONTRACT_REFERENCE.md`, the indexer migration, and the TypeScript bindings
- [ ] Error code changes update `CONTRACT_REFERENCE.md`, the README error tables, and downstream clients are notified
- [ ] TTL constant changes are reviewed for production ledger cadence impact
- [ ] Storage key additions are reviewed for key collision potential and documented in the `DataKey` enum comments
- [ ] `docs/CONTRACT_REFERENCE.md` and `ai.md` are updated for new functions, events, error codes, or env vars

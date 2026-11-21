# Test coverage

CI (`.github/workflows/coverage-report.yml`) measures coverage for the Rust
workspace and the Node backend, uploads the reports as workflow artifacts
(`rust-coverage`, `backend-coverage`) and prints a summary in the job summary.

## Run locally

Rust (needs `cargo install cargo-llvm-cov` and the `llvm-tools-preview` component):

```
cargo llvm-cov --workspace --exclude scoutchain-chaos-tests \
  --exclude migration-e2e --lcov --output-path lcov.info
cargo llvm-cov report --summary-only
```

Backend:

```
npx jest --config jest.coverage.config.js --coverage
```

## Thresholds

Thresholds are a floor that must never decrease:

- Rust: `RUST_MIN_LINE_COVERAGE` in the coverage workflow (line coverage, percent).
- Backend: `coverageThreshold` in `jest.coverage.config.js`.

The initial values are conservative placeholders because the baseline could not
be measured when the job was added. After the first CI run, set each threshold
to the reported value (rounded down) and only raise it afterwards.

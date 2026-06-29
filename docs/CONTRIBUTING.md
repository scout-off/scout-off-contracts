# Contributing

## Prerequisites

Ensure the following tools are installed at the specified minimum versions before attempting to build or test the contracts. Mismatched versions are the most common cause of opaque WASM build failures.

| Tool | Minimum version | Install / notes |
|------|----------------|-----------------|
| **Rust** (via rustup) | stable (1.78+) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **WASM build target** | `wasm32v1-none` | `rustup target add wasm32v1-none` |
| **cargo** | ships with Rust stable | Verify: `cargo --version` |
| **clippy** | ships with Rust stable | `rustup component add clippy` |
| **rustfmt** | ships with Rust stable | `rustup component add rustfmt` |
| **stellar-cli** | latest stable | See [Stellar CLI install guide](https://developers.stellar.org/docs/tools/developer-tools/cli/install-stellar-cli) |
| **Node.js** | 20 LTS | Required only for TypeScript bindings generation — `./scripts/generate-bindings.sh` |
| **npm** | 10+ (ships with Node 20) | Required only for building/testing bindings packages |

> CI uses `dtolnay/rust-toolchain@stable` and installs `stellar-cli` from the latest `main` install script. If a local build diverges from CI, update your Rust toolchain (`rustup update stable`) and reinstall stellar-cli.

The `wasm32v1-none` target (not the older `wasm32-unknown-unknown`) is required for building Soroban contracts with `soroban-sdk 25.x`. Using the wrong target produces an ABI-incompatible WASM binary.

## Setup

```bash
rustup target add wasm32-unknown-unknown
rustup component add clippy rustfmt
cp .env.example .env
```

## Before opening a PR

```bash
cargo test --workspace          # all tests must pass
cargo clippy --workspace        # zero warnings
cargo fmt --all -- --check      # formatting must be clean
```

## Contract change checklist

- [ ] New functions have unit tests covering the happy path and at least one error case
- [ ] Any new `DataKey` variant is documented with a comment
- [ ] Cross-contract calls are documented with a comment explaining the atomicity guarantee
- [ ] `ai.md` is updated if shared types, events, or env vars changed
- [ ] `docs/CONTRACT_REFERENCE.md` is updated with new functions

## Validator authorization changes

Changes to validator registration, revocation, or milestone approval logic require explicit
review from a second team member before merge — these are the trust anchors of the platform.

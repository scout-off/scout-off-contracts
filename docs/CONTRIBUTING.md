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
| **stellar-cli** | **21.6.0** (pinned) | See install note below |
| **Node.js** | 20 LTS | Required only for TypeScript bindings generation — `./scripts/generate-bindings.sh` |
| **npm** | 10+ (ships with Node 20) | Required only for building/testing bindings packages |

> CI uses `dtolnay/rust-toolchain@stable` and installs `stellar-cli` at the pinned version listed above. If a local build diverges from CI, update your Rust toolchain (`rustup update stable`) and reinstall stellar-cli at the pinned version.

### Installing the pinned stellar-cli version

`scripts/generate-bindings.sh` enforces the required `stellar-cli` version and will fail with a
clear error if the wrong version is detected. Install the exact version with:

```bash
curl -sSL https://raw.githubusercontent.com/stellar/stellar-cli/v21.6.0/install.sh | bash
```

Then verify: `stellar --version` should print `stellar 21.6.0`.

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
- [ ] New admin-gated functions use the shared `scoutchain_shared_types::require_admin` helper (see the `AdminError` trait) rather than hand-writing inline admin-check logic

### Error variant ordering

Every contract's `#[contracterror]` enum (`errors.rs`) is append-only.
**New error variants must be added at the end of the enum, never inserted
between existing variants and never renumbered.** This matches the
`docs/VERSIONING.md` policy: renumbering or removing an existing
`#[contracterror]` variant is a MAJOR breaking change because external
consumers, on-chain event listeners, and off-chain indexers key off the
numeric code.

When adding a new variant:

- Append it after the last existing variant. Do not "fill gaps" in the
  numeric sequence — a gap (e.g. `12 → 14`) is a deliberate reservation,
  not a bug, and must be preserved.
- Group related variants together by inserting a brief section comment
  above the group (e.g. `// ── Rate limiting ──`). Grouping is purely
  cosmetic for readers; numeric contiguity within a group is **not**
  required and **not** guaranteed by this convention.
- If the new variant belongs to an existing group, place it at the end
  of that group rather than at the end of the enum, so the grouping
  remains readable. This does not violate append-only because the
  variant's numeric code is the next free value after the current
  maximum — readers can still scan the file top-to-bottom to find it.
- Do not reuse a numeric code that has been removed in a prior version,
  even if the variant is long-since deprecated. On-chain history may
  still reference it.

Rationale and the full set of breaking-change rules live in
[`docs/VERSIONING.md`](VERSIONING.md).

## Validator authorization changes

Changes to validator registration, revocation, or milestone approval logic require explicit
review from a second team member before merge — these are the trust anchors of the platform.

## Glossary

Unfamiliar with terms like *validator*, *milestone*, *subscription tier*, or *CID*?
See [docs/GLOSSARY.md](GLOSSARY.md) for authoritative definitions of all domain-specific terms.

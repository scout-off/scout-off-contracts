# Contributing

## Prerequisites

Ensure the following tools are installed at the specified minimum versions before attempting to build or test the contracts. Mismatched versions are the most common cause of opaque WASM build failures.

| Tool | Minimum version | Install / notes |
|------|----------------|-----------------|
| **Rust** (via rustup) | pinned in `rust-toolchain.toml` | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **WASM build target** | `wasm32v1-none` | `rustup target add wasm32v1-none` |
| **cargo** | ships with Rust stable | Verify: `cargo --version` |
| **clippy** | ships with Rust stable | `rustup component add clippy` |
| **rustfmt** | ships with Rust stable | `rustup component add rustfmt` |
| **stellar-cli** | **25.2.0** (pinned) <!-- Keep in sync with: scripts/generate-bindings.sh and .github/workflows/contract-ci.yml --> | See install note below |
| **Node.js** | 20 LTS | Required only for TypeScript bindings generation — `./scripts/generate-bindings.sh` |
| **npm** | 10+ (ships with Node 20) | Required only for building/testing bindings packages |

> The repository includes `rust-toolchain.toml`, so `rustup` automatically selects the same pinned Rust version, `wasm32v1-none` target, and formatter/linter components used by CI whenever you run `cargo` or `rustup` from this directory. If a local build diverges from CI, reinstall stellar-cli at the pinned version.

### Installing the pinned stellar-cli version

`scripts/generate-bindings.sh` enforces the required `stellar-cli` version and will fail with a
clear error if the wrong version is detected. Install the exact version with:

```bash
# Keep in sync with: scripts/generate-bindings.sh and .github/workflows/contract-ci.yml
curl -sSL https://raw.githubusercontent.com/stellar/stellar-cli/v25.2.0/install.sh | bash
```

Then verify: `stellar --version` should print `stellar 25.2.0`.

The `wasm32v1-none` target (not the older `wasm32-unknown-unknown`) is required for building Soroban contracts with `soroban-sdk 25.x`. Using the wrong target produces an ABI-incompatible WASM binary.

## Setup

```bash
rustup show
rustup component add clippy rustfmt
cp .env.example .env

# (Optional) Install local git pre-push hook to run fmt, clippy, and docs checks
git config core.hooksPath scripts/git-hooks
```

Reminder: `core.hooksPath` is a per-clone git config setting, so re-apply it after every fresh clone or when working from a new machine.

## Before opening a PR

```bash
cargo test --workspace          # all tests must pass
cargo clippy --workspace        # zero warnings
cargo fmt --all -- --check      # formatting must be clean
bash scripts/check-docs.sh      # documentation completeness check
bash scripts/check-event-topic-consistency.sh  # event-topic / docs consistency
bash scripts/check-error-code-continuity.sh  # append-only error code continuity
bash scripts/check-cargo-doc.sh  # public-item docs coverage check
```

## CI checks

The repository defines seven CI jobs across `.github/workflows/ci.yml` and `.github/workflows/contract-ci.yml`. The table below lists each job, its purpose, and whether it is configured as a **required** status check (i.e., blocks merging to `main`) per GitHub's branch-protection rules.

| Job | File | What it checks | Required |
|-----|------|----------------|----------|
| `check-todos` | `ci.yml` | Scans `contracts/` for `TODO`/`FIXME`/`HACK`/`XXX` markers — fails if any are found | Yes |
| `test` | `contract-ci.yml` | Runs `cargo test --workspace` (including each contract's `tests/cost_budget.rs` CPU-instruction cost budget), tests `scoutchain-progress`, uploads a `cpu-cost-budget-<sha>` report artifact, builds WASM release | Yes |
| `lint` | `contract-ci.yml` | Clippy (deny warnings), `rustfmt` check, shellcheck on all tracked shell scripts, docs completeness (`scripts/check-docs.sh`), event-topic consistency (`scripts/check-event-topic-consistency.sh`), guard-ordering audit (`scripts/check-guard-ordering.sh`), cross-doc consistency (`scripts/check-cross-doc-consistency.sh`), trial-offer flow doc consistency (`scripts/check-trial-offer-flow-consistency.sh`), error-code continuity (`scripts/check-error-code-continuity.sh`), public-item docs coverage (`scripts/check-cargo-doc.sh`), and bindings template validation (`scripts/check-bindings.sh`) | Yes |
| `bindings-smoke-test` | `contract-ci.yml` | Deploys all contracts to a local Soroban sandbox, initializes them and wires cross-contract links, verifies that wiring (`scripts/verify-cross-contract-wiring.sh`), runs `scripts/health-check.sh` and `scripts/full-readiness-check.sh`, runs `scripts/smoke-test.sh`, generates TypeScript bindings, verifies their structure, builds each binding package, **verifies that every ABI-declared function has a corresponding export in the generated binding** (fails with a clear list of missing exports if any), runs `scripts/migrate-contract-smoke-test.sh` against the already-running sandbox to continuously verify the deploy-old→seed→migrate→replay→before/after-comparison path, and re-runs `scripts/check-docs.sh` | Yes |
| `abi-export` | `contract-ci.yml` | Exports contract ABIs to `abi/*.json` using `stellar contract info interface`, validates JSON parseability, measures each contract's optimized WASM size against `ci/wasm-size-budget.json` (failing the job if any contract is over budget), and uploads the artifacts; per `docs/VERSIONING.md` the ABI diff is how breaking changes are detected | Yes |
| `abi-diff` | `contract-ci.yml` | Diffs the PR branch's ABI against the base branch (main/develop), classifies changes as MAJOR/MINOR/PATCH per `docs/VERSIONING.md`, and fails if a MAJOR or MINOR change lacks a matching entry in `CHANGELOG.md`'s Unreleased section and a matching row in `docs/VERSIONING.md`'s Version History table | Yes |
| `budget-calibration` | `contract-ci.yml` | Downloads the `abi-export` job's ABI and CPU-cost-budget artifacts, runs `scripts/calibrate-budgets.py --headroom 0.20` to check the checked-in size/cost budgets aren't badly out of calibration with measured values, and fails if any measured value exceeds its budget | Not verifiable from this repo alone — configure via `Settings > Branches > main > Require status checks` and confirm here |

> **Note on the audit:** The "What it checks" column for every job above was re-audited directly against the live job definitions in `.github/workflows/ci.yml` and `.github/workflows/contract-ci.yml`, not just against branch-protection status. The `Required` column reflects the actual branch-protection rules on `main` at the time of writing, except for `budget-calibration`, whose required-status could not be confirmed while auditing this table (see that row). Because changing branch-protection settings requires repository admin access, any future update to the required checks — including confirming `budget-calibration`'s status — must be performed by a maintainer in the repository settings (`Settings > Branches > main > Require status checks`).

### Why `abi-export` is required

Per `docs/VERSIONING.md`, the ABI export exists specifically so that reviewers can diff the output across commits to detect breaking changes. Making it a required check ensures no PR can merge without a fresh ABI artifact being generated and examined.

### WASM size and CPU-cost budgets

The release profile (`opt-level = "z"`, `lto = true`, `codegen-units = 1`) exists because Soroban charges resource fees proportional to CPU instructions and ledger I/O, and enforces hard per-transaction and per-ledger-entry size limits. Two checked-in budgets guard against silent regressions:

- **WASM binary size** — `ci/wasm-size-budget.json`, enforced by the `abi-export` job's "Check WASM size budget" step against each contract's `stellar contract optimize` output.
- **CPU-instruction cost** — `ci/cpu-cost-budget.md` (source of truth: the `*_CPU_BUDGET` constants in each `contracts/<name>/tests/cost_budget.rs`), enforced as part of `cargo test --workspace` in the `test` job.

Both files document their own process for intentionally raising a budget when a legitimate feature grows a contract's size or an operation's cost — bump the number and add a one-line justification in the PR description.

## Contract change checklist

- [ ] New functions have unit tests covering the happy path and at least one error case
- [ ] Any new `DataKey` variant is documented with a comment
- [ ] Cross-contract calls are documented with a `**Cross-contract calls:**` row in the
  function's `CONTRACT_REFERENCE.md` entry and a comment explaining the atomicity guarantee
- [ ] `ai.md` is updated if shared types, events, or env vars changed
- [ ] `docs/CONTRACT_REFERENCE.md` is updated with new functions, events, and error codes *(enforced automatically by `scripts/check-docs.sh` in the CI lint job — the PR will fail if a `pub fn` from any `#[contractimpl]` block lacks a corresponding heading in the docs)*

### Error variant ordering

Every contract's `#[contracterror]` enum (`errors.rs`) is append-only.
**New error variants must be added at the end of the enum, never inserted
between existing variants and never renumbered.** This matches the
`docs/VERSIONING.md` policy: renumbering or removing an existing
`#[contracterror]` variant is a MAJOR breaking change because external
consumers, on-chain event listeners, and off-chain indexers key off the
numeric code.

`scripts/check-error-code-continuity.sh` is the automated backstop for this
policy. It compares every `errors.rs` in the PR branch against `origin/main`
and fails if any numeric code present in both has a different variant name, or
if a code that existed in `main` disappears without an explicit `// reserved`
comment in the new version. This check is now part of the `lint` job in
`contract-ci.yml`, and it should still be run locally before opening a PR that
touches an `errors.rs` file as a fast, targeted validation of the same policy.

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
The validator contract is covered by [`.github/CODEOWNERS`](../.github/CODEOWNERS), which
requests review from the designated validator-logic owner for changes under
`/contracts/verification/`.

Beyond `/contracts/verification/`, [`.github/CODEOWNERS`](../.github/CODEOWNERS) also designates
owners for `/contracts/registration/`, `/contracts/progress/`, `/contracts/scout_access/`, and a
`*` default covering every remaining path (`docs/`, `scripts/`, `config/`, `migrations/`,
`bindings/`, and top-level files). The same **Require review from Code Owners** branch-protection
rule therefore gates changes to those paths once it is enabled by an administrator — there are no
intentionally owner-less areas of the repository. Owners may be redirected in review if a different
reviewer is more appropriate for a given contract.

Repository administrators must enable **Require review from Code Owners** in the `main`
branch-protection rule for this mapping to block merges. Before enabling that rule, confirm
that the listed owner has the required write access and update the mapping if the authorized
reviewer group changes.

## Glossary

Unfamiliar with terms like *validator*, *milestone*, *subscription tier*, or *CID*?
See [docs/GLOSSARY.md](GLOSSARY.md) for authoritative definitions of all domain-specific terms.

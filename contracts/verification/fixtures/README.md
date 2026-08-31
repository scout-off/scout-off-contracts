# `scoutchain_progress.wasm` fixture

## What this file is

`scoutchain_progress.wasm` is a compiled **WASM build of the
`scoutchain-progress` contract** (`contracts/progress/`). It is *not* a
hand-written mock: it is the real contract compiled for Soroban so that the
`scoutchain-verification` contract can generate a **type-safe client for its
cross-contract calls**.

It is consumed at compile time by this crate in
`src/lib.rs`:

```rust
mod progress_contract {
    soroban_sdk::contractimport!(file = "fixtures/scoutchain_progress.wasm");
}
```

`soroban_sdk::contractimport!` reads the wasm, parses its interface (the
`advance_level` function plus the `ProgressError` return type, including the
exact error *discriminants*), and generates the `progress_contract::Client`
and `progress_contract::ProgressError` types that `commit_approved_milestone` uses to
invoke `try_advance_level` and to special-case `AlreadyAtMaxLevel` (see
`src/lib.rs`, the private helper `commit_approved_milestone` called by
`approve_milestone` / `attest_milestone` / `submit_attested_milestone`).

Because the client mirrors the *real* deployed contract's ABI, tests and
cross-contract wiring here are guaranteed to line up with what the production
`scoutchain-progress` contract exports. **Do not replace this binary with a
hand-written `.wasm` or a synthetic mock** unless you can guarantee the
generated client's `ProgressLevel`/`ProgressError` encodings (including
discriminant values) are byte-identical to the deployed contract.

## Where it was built from

- **Source crate:** `contracts/progress/` (package `scoutchain-progress`)
- **Crate version:** `1.1.0` (`version.workspace` — see root `Cargo.toml`)
- **Target:** `wasm32v1-none` (the Soroban WASM target pinned in
  `rust-toolchain.toml`, channel `1.97.1`)
- **Profile:** release (workspace `[profile.release]` —
  `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`)
- **Toolchain:** channel `1.97.1` per `rust-toolchain.toml`

### Git provenance

Because `*.wasm` is git-ignored (root `.gitignore`), this binary can only be
committed with `git add -f`. Its current committed state comes from:

```
e51e415e8bc7e9e2d4c03fd68554065e4f357d27  2026-06-20  fix: move Admin to persistent storage and add upgrade() to all contracts
```

That commit is the point the fixture's compiled interface was last refreshed.
It is not a substitute for the authoritative fingerprint of a freshly built
artifact from the same commit — see "Regenerating" below and compare SHA-256s
when in doubt.

### Current committed artifact fingerprint

```
SHA-256: 9b6750e03a681c19dc5c5cc0a494055b9d1fcfba6e9e5439a5d0503abb877686
```

## Regenerating

Whenever the `scoutchain-progress` public interface changes — for example,
`advance_level`'s signature, `ProgressLevel`, or any `ProgressError` variant
or its `#[repr]` discriminant values — regenerate the fixture and commit the
updated binary so the verification client stays in sync.

From the repository root:

```bash
cargo build -p scoutchain-progress --target wasm32v1-none --release
cp target/wasm32v1-none/release/scoutchain_progress.wasm \
   contracts/verification/fixtures/scoutchain_progress.wasm
```

This is the same build used by `scripts/smoke-test.sh` (`cargo build
--workspace --target wasm32v1-none --release`, artifact
`target/wasm32v1-none/release/scoutchain_progress.wasm`). Note that the
fixture is the **pre-optimization release build** — do not run
`stellar contract optimize` on it; optimization is a deployment-time step
that would emit a different binary than the interface codegen expects.

After copying, verify the interface regenerates cleanly:

```bash
cargo check -p scoutchain-verification --target wasm32v1-none
```

### Committing the regenerated fixture

The file is ignored by git, but it is *already tracked*, so updates stage
normally with `git add contracts/verification/fixtures/scoutchain_progress.wasm`.
Only a fresh checkout that has lost the binary (e.g. a `.gitattributes`
change or a re-base that dropped the tracked blob) would need a force-add:

```bash
git add -f contracts/verification/fixtures/scoutchain_progress.wasm
```

## When editing the progress contract

1. Make your change in `contracts/progress/src/`.
2. Regenerate the fixture (above).
3. Update this file's **SHA-256** and the **Git provenance** note to point at
   the commit that contains the change, so future readers can tell at a glance
   how fresh the artifact is.
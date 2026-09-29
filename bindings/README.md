# ScoutChain — TypeScript Contract Bindings

> **These are generated artifacts.** The `src/` directory and compiled `dist/`
> output inside each binding package are produced by
> `stellar contract bindings typescript` after a live contract deployment.
> They are **not** committed to the repository. What *is* committed is the
> `package.json` and `tsconfig.json` scaffold in each subdirectory so the
> packages are immediately `npm install`-able once generation runs.

---

## Prerequisites

Before you can generate bindings you need:

1. **Deployed contracts** — run `./scripts/deploy.sh testnet` (or `mainnet`).
   This writes contract IDs to `.env.contracts`.
2. **Initialized contracts** — run `./scripts/initialize.sh testnet`.
3. **Stellar CLI** installed (exact pinned version **25.2.0** required) — see
   [Installing the pinned stellar-cli version](../docs/CONTRIBUTING.md#installing-the-pinned-stellar-cli-version) in `docs/CONTRIBUTING.md` for detailed installation instructions.
4. **`.env.contracts`** present and containing all four non-empty IDs:
   ```
   REGISTRATION_CONTRACT_ID=C...
   VERIFICATION_CONTRACT_ID=C...
   PROGRESS_CONTRACT_ID=C...
   SCOUT_ACCESS_CONTRACT_ID=C...
   ```

---

## Generate

```bash
# Testnet (default)
./scripts/generate-bindings.sh testnet

# Mainnet
./scripts/generate-bindings.sh mainnet
```

The script validates that `.env.contracts` exists and all four IDs are
non-empty before making any network calls. It exits immediately with a
descriptive error if anything is missing.

### Version sync

Each binding package's `package.json` `version` is kept in lockstep with the
contracts' workspace version — the value of `[workspace.package].version` in
[`Cargo.toml`](../Cargo.toml), the build-time source of truth documented in
[`docs/VERSIONING.md`](../docs/VERSIONING.md). The committed scaffolds already
carry that version, and `scripts/generate-bindings.sh` re-derives it from
`Cargo.toml` on every run and rewrites each generated `package.json` after the
CLI (which overwrites the package with its own template and a placeholder
version). Bump the version in `Cargo.toml` and the bindings follow
automatically on the next regeneration.

---

## Build

After generation each package must be compiled before it can be imported:

```bash
cd bindings/registration  && npm install && npm run build && cd ../..
cd bindings/verification  && npm install && npm run build && cd ../..
cd bindings/progress      && npm install && npm run build && cd ../..
cd bindings/scout_access  && npm install && npm run build && cd ../..
```

Or in one line from the repo root:

```bash
for pkg in registration verification progress scout_access; do
  (cd "bindings/$pkg" && npm install && npm run build)
done
```

---

## Structure

```
bindings/
  registration/
    README.md        ← package overview and links to full documentation
    package.json     ← committed — package scaffold (overwritten by CLI on generation)
    tsconfig.json    ← committed — TypeScript compiler config
    src/             ← GENERATED — do not edit; re-run generate-bindings.sh to refresh
      index.ts
    dist/            ← BUILT — output of `npm run build`; gitignored
  verification/      ← same structure
  progress/          ← same structure
  scout_access/      ← same structure
```

---

## Use in backend / frontend

Install from the local path:

```bash
# from your backend or frontend repo root
npm install file:../scout-off-contracts/bindings/registration
npm install file:../scout-off-contracts/bindings/verification
npm install file:../scout-off-contracts/bindings/progress
npm install file:../scout-off-contracts/bindings/scout_access
```

Or publish to a private npm registry and install by name:

```bash
npm install @scoutchain/bindings-registration
npm install @scoutchain/bindings-verification
npm install @scoutchain/bindings-progress
npm install @scoutchain/bindings-scout-access
```

Then import in your code:

```typescript
import { Client as RegistrationClient } from "@scoutchain/bindings-registration";
import { Client as VerificationClient } from "@scoutchain/bindings-verification";
import { Client as ProgressClient }     from "@scoutchain/bindings-progress";
import { Client as ScoutAccessClient }  from "@scoutchain/bindings-scout-access";
```

Instantiate with your RPC URL and the network constants exported by each
package:

```typescript
import * as Registration from "@scoutchain/bindings-registration";

const client = new Registration.Client({
  ...Registration.networks.testnet, // includes contractId and networkPassphrase
  rpcUrl: process.env.SOROBAN_RPC_URL!,
});

const player = await client.get_player({ player_id: 1n });
```

---

## Auto-Generated Function Lists

Each `bindings/<contract>/README.md` contains a clearly-delimited
**auto-generated function list** section, injected by
`scripts/generate-bindings.sh` at binding-generation time.

### What it contains

The section is extracted from [`docs/CONTRACT_REFERENCE.md`](../docs/CONTRACT_REFERENCE.md)
and lists every public function in the contract with its full signature and
a one-line description. It is delimited by HTML comment markers so it can be
located and replaced reliably on each run:

```html
<!-- AUTO-GENERATED FUNCTION LIST BEGIN - DO NOT EDIT MANUALLY -->
...function list...
<!-- AUTO-GENERATED FUNCTION LIST END -->
```

### Idempotent re-generation

Re-running `./scripts/generate-bindings.sh` is idempotent: if the function
list section already exists in a README, it is replaced in-place with the
current content extracted from `CONTRACT_REFERENCE.md`. If `CONTRACT_REFERENCE.md`
has not changed, the README is byte-for-byte identical after a second run.

### Manually maintained content

All content **outside** the delimited block is manually maintained and will
not be touched by the generation script. Add package-specific notes, usage
examples, or integration guidance outside the auto-generated block.

### Source of truth

`CONTRACT_REFERENCE.md` is the authoritative source for function documentation.
The auto-generated block in each bindings README is a convenience summary.
For complete documentation including parameters, return types, authorization
requirements, and CLI examples, always refer to `CONTRACT_REFERENCE.md`.

---

## Gitignore

The generated `src/` directories and compiled `dist/` output are excluded
from version control. Only the `package.json` and `tsconfig.json` scaffolds
are committed.

```gitignore
# in .gitignore
bindings/*/src/
bindings/*/dist/
bindings/*/node_modules/
```

---

## Troubleshooting

| Error | Cause | Fix |
|-------|-------|-----|
| `.env.contracts not found` | `deploy.sh` has not been run | `./scripts/deploy.sh testnet` |
| `REGISTRATION_CONTRACT_ID is empty` | Deploy failed or `.env.contracts` is stale | Re-run `./scripts/deploy.sh testnet` |
| `Cannot find module '@stellar/stellar-sdk'` | `npm install` not run after generation | `cd bindings/<name> && npm install` |
| `dist/` missing | `npm run build` not run | `cd bindings/<name> && npm run build` |
| Stale types after contract change | Contract redeployed but bindings not regenerated | `./scripts/generate-bindings.sh testnet` then rebuild |
| Wrong `stellar-cli` version | Local CLI does not match the pinned version | Install the pinned version in [`docs/CONTRIBUTING.md`](../docs/CONTRIBUTING.md#installing-the-pinned-stellar-cli-version) |

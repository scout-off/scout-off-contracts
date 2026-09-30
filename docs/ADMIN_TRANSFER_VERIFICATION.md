# Admin Transfer Verification

This document records the property-based test suite added in issue #824 to
formally verify the `propose_admin` / `accept_admin` two-step flow across all
four contracts: `registration`, `verification`, `progress`, and `scout_access`.

## Invariants under test

| # | Property | Description |
|---|----------|-------------|
| 1 | **Accept-by-proposed-only** | `accept_admin` succeeds only when called by the exact address most recently passed to `propose_admin`. Any other caller is rejected. |
| 2 | **Double-propose replaces** | Calling `propose_admin` twice replaces the pending proposal; it does not queue or merge. The second proposed address is the only one that can accept. |
| 3 | **Admin immutability** | The `Admin` storage value is unchanged until a successful `accept_admin` call. No other function mutates it. |
| 4 | **Replaced-proposal rejection** | After a pending proposal is replaced by a newer `propose_admin`, the old proposed address can no longer accept. |

## Test files

- `contracts/registration/tests/admin_transfer_properties.rs`
- `contracts/verification/tests/admin_transfer_properties.rs`
- `contracts/progress/tests/admin_transfer_properties.rs`
- `contracts/scout_access/tests/admin_transfer_properties.rs`

Each file exercises the same four properties against its contract's generated
client.

## Running the suite

```bash
cargo test --workspace --test admin_transfer_properties
```

## Re-verification after changes

Any change to `propose_admin`, `accept_admin`, or the `PendingAdmin` storage
key in any contract must re-run this suite.  If a new divergence between
contracts is discovered, fix it to match the intended invariant set and add a
regression test in the same file.

## Results

All four contracts pass all four properties as of the issue #824 implementation
commit.

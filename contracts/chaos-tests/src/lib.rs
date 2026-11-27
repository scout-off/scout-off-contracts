//! ScoutChain chaos-test crate.
//!
//! Integration tests under `tests/` own the fixtures, schedule generator,
//! and invariant checkers (they require `soroban-sdk/testutils`).
//!
//! The crate has no library code of its own. `#![no_std]` keeps the empty
//! lib buildable for the `wasm32v1-none` contract target that the workspace
//! WASM build (`cargo build --workspace --target wasm32v1-none`) exercises;
//! the integration tests still run on the host `std` target as normal.
#![no_std]

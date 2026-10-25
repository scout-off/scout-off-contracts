# Contract Reference

Quick reference for the `scout_access` error codes added in #1675 / #1676.
The complete, authoritative list for every contract is in
[docs/CONTRACT_REFERENCE.md](docs/CONTRACT_REFERENCE.md).

## scout_access

### `ScoutAccessError` (additions)

| Code | Variant | Meaning |
| ---- | ------- | ------- |
| 50 | `NotPlayerOwner` | The caller's wallet does not own the supplied `player_id`. |
| 51 | `ProgressContractNotSet` | The progress contract has not been wired into this deployment. |

`ProgressContractNotSet` is returned by `confirm_trial_offer` when the progress
contract is not configured. This is a deployment-wiring problem, not a problem
with the caller's input; integrators should check contract wiring rather than
their request payload.

# Contract Reference

This document lists the public error codes and reference material for the
contracts in this repository.

## scout_access

### `ScoutAccessError`

| Code | Variant | Meaning |
| ---- | ------- | ------- |
| 1 | `Unauthorized` | Caller is not authorized for this action. |
| 2 | `NotFound` | Requested resource does not exist. |
| 3 | `AlreadyExists` | Resource already exists. |
| 4 | `InvalidInput` | Caller-supplied input is invalid. |
| 5 | `RegistrationContractNotSet` | The registration contract has not been wired into this deployment. |
| 6 | `ProgressContractNotSet` | The progress contract has not been wired into this deployment. |

`ProgressContractNotSet` is returned by `confirm_trial_offer` when the progress
contract is not configured. This is a deployment-wiring problem, not a problem
with the caller's input; integrators should check contract wiring rather than
their request payload.

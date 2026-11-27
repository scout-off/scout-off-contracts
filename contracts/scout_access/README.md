# Scout Access Contract

The Scout Access contract manages scout registration, player ownership verification, and evidence access grants for the scouting system.

## Overview

Scouts register with the contract, verify ownership of player records, and request access to player evidence. Players (or their owners) grant or revoke evidence access. Admins can also revoke evidence access.

## Error Reference

| Error | When it is returned |
| --- | --- |
| `RegistrationContractNotSet` | A registration-dependent operation (e.g. `verify_player_ownership`) is called before the registration contract has been wired. This is a wiring/configuration error, not a verification failure. |
| `PlayerNotVerified` | The player's verification status is not valid for the requested operation (e.g. the player record is not verified). |
| `NotPlayerOwner` | The caller's wallet does not own the supplied `player_id`. Distinct from `PlayerNotVerified` so callers can tell an ownership mismatch apart from a verification failure. |
| `GrantAlreadyRevoked` | Returned by `revoke_evidence_access` when the grant was already revoked. See the idempotency rule below. |

## Revoke Idempotency

Both revoke paths follow the same documented rule:

- `admin_revoke_evidence_access` is **idempotent**: revoking an already-revoked grant returns `Ok(())`.
- `revoke_evidence_access` (player path) is **not** idempotent: revoking an already-revoked grant returns `GrantAlreadyRevoked`.

Callers should treat `GrantAlreadyRevoked` from the player path as a benign no-op if they only care about the end state (grant revoked). The admin path can be retried safely without inspecting the result.

## Ownership Verification

`verify_player_ownership` distinguishes two failure modes:

- If the registration contract is not wired, it returns `RegistrationContractNotSet`.
- If the wallet does not own `player_id`, it returns `NotPlayerOwner`.

This lets callers surface accurate diagnostics instead of a generic verification error.

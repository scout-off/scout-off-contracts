# Validator Collusion Threat Model

## Threat

A malicious operator can control many wallets and have each wallet registered as
an otherwise valid validator. Credentials are display text and, by themselves,
do not establish independence. The operator can then approve fabricated
milestones from multiple wallets for the same player.

With the existing five-milestone-per-validator-per-player policy and a
100-validator registry cap, the maximum stated blast radius is 500 approvals
for one player (5 approvals × 100 colluding wallets). A per-wallet cap limits
one key, not a coordinated group.

## Organizational diversity control

Each validator now has an admin-set `affiliation` in addition to its display
`credentials`. Before registration, the administrator must verify the
organization using the platform's off-chain onboarding process and use one
canonical affiliation value for every validator representing that organization.
Changing names or creating more wallets must not result in a new affiliation
without separate verification.

For each player, the contract records whether an affiliation has contributed a
milestone and maintains the count of distinct affiliations. It never counts two
wallets with the same affiliation twice.

## Advancement rule

The default configuration requires two distinct affiliations starting with the
second milestone, which is the milestone eligible to advance a player to level
2. The administrator may set both values with `set_diversity_config`.

- Milestones are always retained as an audit record.
- Before the gated milestone, normal advancement is allowed.
- At and after the gate, `approve_milestone` calls the progress contract only
  when the player has the configured number of distinct affiliations.
- A later independent affiliation can satisfy the requirement and allow the
  next eligible milestone to advance progress.

This limits on-chain Sybil wallets but does not prove real-world independence:
the administrator's affiliation verification remains the trust anchor and must
be reviewed and audited.

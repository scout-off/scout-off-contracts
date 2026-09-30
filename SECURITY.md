# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| `main` (testnet) | ✅ Active development |
| Older branches | ❌ Not supported |

For deployed contracts, only the **latest released contract version** is patched.
If a vulnerability is found in a deployed-but-not-latest version, the fix ships as a
patch on the latest minor only — operators running older versions are expected to
upgrade to the latest minor to receive the fix. See
[docs/VERSIONING.md](docs/VERSIONING.md) for the versioning policy (SemVer
MAJOR/MINOR/PATCH semantics and upgrade procedures).

> **Testnet note:** The currently deployed testnet contracts are under active
development and are upgraded in place as releases land; no backporting is performed
for older versions. This policy will be revisited when a mainnet deployment is
introduced.

## Reporting a Vulnerability

We take the security of ScoutChain and its smart contracts seriously. If you believe you have discovered a security vulnerability, please report it to us privately.

### Primary channel — GitHub Private Vulnerability Reporting (recommended)

Use [GitHub's built-in Private Vulnerability Reporting](https://github.com/scout-off/scout-off-contracts/security/advisories/new)
to submit a vulnerability report. This channel is **live and monitored** and is the
preferred path for all responsible disclosures.

1. Go to the [scout-off/scout-off-contracts](https://github.com/scout-off/scout-off-contracts) repository
2. Navigate to **Settings** → **Security** → **Private vulnerability reporting** (or use the "Report a vulnerability" link under the repository's Security tab)
3. Submit a detailed report describing the vulnerability, including:
   - The affected contract(s) and function(s)
   - Steps to reproduce the issue
   - Potential impact and exploit scenario
   - Any suggested remediation (if known)

### Secondary channel — email

> ⚠️ **PLACEHOLDER — NOT YET OPERATIONAL**
>
> `security@scout-off.io` is listed below as a secondary contact address, but it is
> **not yet a live, monitored inbox**. It **must not** be relied upon as a real
> reporting channel until a team member has confirmed the inbox is active and
> monitored.
>
> **Before this responsible-disclosure process is used for any real vulnerability
> report, the following action must be completed:**
>
> - [ ] Stand up and verify `security@scout-off.io` as a monitored mailbox, then
>       remove this warning block and update the status below.
>
> Tracked in: [scout-off/scout-off-contracts #879](https://github.com/scout-off/scout-off-contracts/issues/879)

Email: `security@scout-off.io` *(placeholder — monitored when operational)*

Until `security@scout-off.io` is confirmed operational, please use **GitHub Private
Vulnerability Reporting exclusively** for all security disclosures.

**Please do not** open public GitHub issues, Discord threads, or support tickets for security vulnerabilities.

---

## Response Timeline

| Milestone | Target |
|-----------|--------|
| Acknowledgement | Within 48 hours (GitHub PVR only while email is not live) |
| Initial assessment | Within 7 days |
| Patch / mitigation | Depends on severity; critical issues prioritised |

## Response Commitments

We aim to acknowledge receipt of vulnerability reports within the following timeframes:

| Severity | Initial Acknowledgment | Target Remediation Timeline |
|----------|----------------------|-----------------------------|
| **Critical** | Within 24 hours | Emergency patch as soon as possible |
| **High** | Within 48 hours | Patch within 7 days |
| **Medium** | Within 5 days | Patch within 30 days |
| **Low** | Within 14 days | Patch within 90 days or next release |

*Timeframes are measured from the initial report submission and assume sufficient information to reproduce and validate the issue.*

---

## Scope

The following are in scope:

- All Soroban smart contracts under `contracts/`
- Deployment and initialization scripts under `scripts/`
- TypeScript binding packages under `bindings/`

The following smart contracts are **in scope** for security reports:

| Contract | Purpose |
|----------|---------| 
| `registration` | Player & scout on-chain identity management |
| `verification` | Validator registry & milestone approvals |
| `progress` | Four-tier progress level state machine |
| `scout_access` | Subscriptions, pay-to-contact, trial offers |

Supporting infrastructure (bindings, scripts, configuration, documentation) is **out of scope** unless a vulnerability in those components directly impacts contract security.

Out of scope:

- Third-party dependencies (report directly to the upstream maintainer)
- Theoretical vulnerabilities with no practical exploit path

---

## Emergency Response: Immediate Mitigation

If you are reporting an **actively exploited critical vulnerability**, refer to the **[Emergency: Pause All Contracts](docs/RUNBOOK.md#emergency-pause-all-contracts)** procedure in the runbook for immediate mitigation steps.

The platform admin can:
1. Run `./scripts/emergency-pause.sh` to halt all state-changing contract operations
2. Verify all four contracts are paused via `health()` queries
3. Coordinate with the reporting researcher on root-cause analysis and remediation

See [`docs/RUNBOOK.md`](docs/RUNBOOK.md) for the full emergency-pause procedure.

For admin key-loss incidents specifically, also see
[`docs/RUNBOOK.md#emergency-admin-key-loss--compromise`](docs/RUNBOOK.md#emergency-admin-key-loss--compromise).

---

## Disclosure Policy

We follow coordinated responsible disclosure. Please do not publicly disclose a
vulnerability until a patch has been released or we have agreed on a disclosure
timeline together.

## Responsible Disclosure Policy

We ask that security researchers:

1. **Report privately first** — Give us a reasonable opportunity to investigate and remediate before any public disclosure
2. **Provide sufficient detail** — Include reproduction steps, affected versions, and proof-of-concept where possible
3. **Act in good faith** — Avoid actions that degrade platform availability, compromise user data, or access production systems beyond what is necessary for proof-of-concept validation
4. **Allow reasonable time for remediation** — Respect the target timelines above before disclosing to third parties

We commit to:

- Acknowledging receipt of your report within the target timeframes above
- Investigating and validating reported issues promptly
- Keeping you informed of remediation progress
- Giving credit for valid, previously unreported vulnerabilities (if desired) in release notes and security advisories

---

## Future Bug Bounty Program

A formal bug bounty program is under evaluation. This policy will be updated when the program launches. In the meantime, we greatly appreciate responsible disclosure and will publicly acknowledge valid reports.

---

## Policy Maintenance

This security policy is reviewed and updated as the platform evolves. Significant changes will be communicated via the repository's release notes and changelog.

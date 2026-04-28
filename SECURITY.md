# Security Policy

## Supported Versions

The latest minor release of `r402` (and the family of crates published from
this repository) is the only line that receives security fixes. Older
versions are upgraded only when the underlying x402 protocol or a
transitive dependency forces it.

| Crate          | Supported releases             |
| -------------- | ------------------------------ |
| `r402`         | `0.14.x` (latest minor only)   |
| `r402-core`    | `0.14.x` (latest minor only)   |
| `r402-evm`     | `0.14.x` (latest minor only)   |
| `r402-svm`     | `0.14.x` (latest minor only)   |
| `r402-http`    | `0.14.x` (latest minor only)   |
| `r402-mcp`     | `0.14.x` (latest minor only)   |

## Reporting a Vulnerability

Please report security issues **privately** through GitHub Security
Advisories on the [`r402` repository][advisories]. This creates a private
draft visible only to the maintainers and the reporter.

If GitHub Security Advisories are unavailable, you may also email the
maintainers at `security@qntx.dev` with the subject prefix
`[r402 security]`. PGP keys for confidential disclosure are listed at the
top of the maintainers' GitHub profiles.

Please include:

- A description of the issue and the impact (confidentiality, integrity,
  availability, financial loss).
- Reproduction steps or proof-of-concept code.
- The affected crate and version, plus any deployment configuration that
  may be relevant.
- Any disclosure deadlines you are working under.

[advisories]: https://github.com/qntx/r402/security/advisories/new

## Coordinated Disclosure

We aim to:

- **Acknowledge** new reports within **3 working days**.
- **Triage and reproduce** within **7 working days**.
- **Ship a fix and advisory** within **30 days** for high-severity issues
  (resource loss, replay, signature spoofing, key disclosure) and within
  **90 days** for lower-severity issues.

If a published advisory is required, we follow a coordinated-disclosure
schedule with the reporter so a fix is in users' hands before the issue
becomes public knowledge.

## Scope

In-scope:

- All crates in this repository (`r402`, `r402-core`, `r402-evm`,
  `r402-svm`, `r402-http`, `r402-mcp`).
- Their public APIs, on-the-wire compatibility with the x402 v2
  specification, and the cross-SDK interoperability surface.
- Cryptographic operations (EIP-712, EIP-3009, Permit2, EIP-2612, EIP-1271,
  EIP-6492, Solana ed25519 signing).
- Smart-wallet signature verification semantics on EVM chains.

Out of scope:

- Issues in upstream third-party libraries (please file these with the
  affected project; we will follow up if a workaround is feasible at the
  r402 layer).
- Vulnerabilities in deployed facilitator services not operated by the
  r402 maintainers.
- Network-layer or DDoS-style availability concerns that do not involve
  protocol-level escalation.

## Hardening Notes

The audit tracked under `audit/` enumerates additional production-readiness
guidance, including private-key zeroisation, tracing PII redaction,
graceful-shutdown semantics, and recommended deployment timeouts. See
`audit/08-security.md` for the full threat model and remediation plan.

# Security Policy

## Reporting a Vulnerability

Open a [GitHub issue](https://github.com/SkillDoAI/llmposter/issues) or email the maintainers directly. We aim to respond within 48 hours.

## Known Advisories

### RUSTSEC-2023-0071 — `rsa` Marvin Attack (timing side-channel)

| Field | Detail |
|-------|--------|
| **Crate** | `rsa 0.9.x` |
| **Via** | `oauth-mock 0.4.4` → `jsonwebtoken` → `rsa` |
| **Severity** | Medium (5.9) |
| **Fix available** | No |
| **Status** | Acknowledged — not applicable |

**Why this doesn't affect llmposter:**
llmposter is a test mock server. The `rsa` crate is used only by `oauth-mock` to sign and verify JWTs in test fixtures. No real private keys, user credentials, or sensitive data are ever processed. The Marvin Attack requires an attacker to measure RSA decryption timing against a long-lived server handling real secrets — none of which apply here.

The advisory is suppressed in `.cargo/audit.toml`. It will be removed when `oauth-mock` or `jsonwebtoken` ships a patched `rsa` dependency.

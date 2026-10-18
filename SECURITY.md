# Security

## Reporting a vulnerability

If the repository's **Security → Report a vulnerability** action is available,
use that private GitHub channel. If it is unavailable, open a minimal issue asking
the maintainers for a private reporting channel, without exploit details or secrets.
There is no published security email or promised response-time agreement.

Include affected revision, a minimal reproduction, impact and any proposed fix.
Do not attach real tokens, pairing records, certificates/keys, provisioning profiles,
personal IPAs or unredacted screen captures. General connection trouble belongs in
an ordinary redacted bug report.

## Supported scope

This is an experimental preview. The current maintenance focus is the latest
Windows GPUI desktop and its shared local daemon/client. Older snapshots and the
legacy desktop do not have a separate long-term support promise.

## Trust boundaries

- The daemon controls a scoped device and requires a bearer token for its API.
- HTTP is plaintext loopback. Use SSH forwarding for a remote hop; do not expose the
  listener directly to an untrusted network.
- A desktop-owned session keeps a temporary token in memory. Exported MCP
  configuration contains that credential and must be handled accordingly.
- Apple's pairing material is local host/device authorization. Do not publish it.
- Signing uses each user's own development identity. The app imports a pre-signed
  runner; it does not collect signing private keys or Apple account credentials.
- Recovery operations require explicit enablement and OS permissions. Uncertain
  mutation outcomes require observation before another attempt, not automatic replay.

Dependency advisories and scoped exceptions are recorded in `.cargo/audit.toml`.
An exception is not a general claim that a dependency is risk-free. Reassess it when
changing TLS, cryptography, pairing or dependency versions. Compilation and hygiene
checks are not a security certification.

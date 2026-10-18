# Contributing to TetherDeck

Thanks for helping improve a Windows-first, open source iPhone workspace.
Reports and discussion in English, Portuguese or Spanish are welcome.

## Where to start

Use [Issues](https://github.com/rafagouveia/TetherDeck/issues) for reproducible bugs,
feature requests and documentation problems. If Discussions is enabled, use it for
open-ended questions and design proposals. Check existing reports before opening a duplicate.
Follow the [community conduct guide](CODE_OF_CONDUCT.md).

A useful bug report includes the app revision/package version, Windows version,
transport, iOS version, expected result, actual result and minimal reproduction.
For performance reports, distinguish GPUI paint FPS, local frame age and API response
from end-to-end latency. Do not present one counter as another measurement.

**Redact before posting:** tokens, UDIDs, serials, host/user names, network addresses,
MCP credentials, screen content and file paths. Never attach pairing records,
private keys, signing passwords, provisioning profiles or personal runner IPAs.
For vulnerabilities, follow [SECURITY.md](SECURITY.md).

## Develop

Read [the README](README.md), [architecture](docs/ARCHITECTURE.md) and the relevant
crate before making changes. Keep a change focused on one problem. Discuss broad
protocol, transport or interaction changes before investing in a large patch.

```powershell
cargo check --locked --workspace
cargo build --release --locked -p tetherdeck-desktop -p rust-ios-device-daemon -p rust-ios-device -p rust-ios-device-mcp
```

This repository's Windows CI runs publication hygiene and compilation checks.
Device-dependent behavior must be described honestly: a successful build does not
prove pairing, installation, touch handling or performance on real hardware.
State which checks and manual observations you actually performed.

## Pull requests

- Describe the problem, change, verification and remaining limitations.
- Preserve single-device scoping, authentication and bounded input/response handling.
- Keep device I/O and decoding off the UI thread. Preserve release/cancellation paths.
- Never replay a device mutation after an uncertain outcome.
- Keep user settings and device-specific data outside the repository.
- Update user-facing docs and translations when behavior changes. Preserve device text.
- For vendored changes, retain upstream licenses, record the exact base revision and
  document the patch. Avoid unrelated reformatting of third-party code.

Do not commit `work/`, `outputs/`, build targets, local caches, recordings or signing
materials. See [publication hygiene](docs/PUBLICATION-CHECKLIST.md).

Original contributions are submitted under the repository's MIT license. Keep
third-party licensing and attribution intact; do not copy code without compatible
permission and provenance. No paid tools or Apple certificate sharing is required
for documentation or source contributions.

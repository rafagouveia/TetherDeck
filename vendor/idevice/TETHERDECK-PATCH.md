# TetherDeck protocol integration patches

## Provenance

- Package: `idevice` 0.1.68, copied from its published Cargo registry source.
- Published source: <https://crates.io/crates/idevice/0.1.68>.
- Upstream: <https://github.com/jkcoxson/idevice>.
- Embedded source revision: `d32c8189c51c2789496b0768039419c3705498c3`, package path `idevice`.
- Published archive SHA-256: `f5d7799c8497cef60ddfd38f7d92e267b9fa5d0b18e7691d31041453007400ca`.
- Declared license: MIT. The published archive omits the license text;
  `LICENSE-MIT` is the unchanged upstream `LICENSE.txt` at that exact revision:
  <https://raw.githubusercontent.com/jkcoxson/idevice/d32c8189c51c2789496b0768039419c3705498c3/LICENSE.txt>.

The workspace selects this source using `[patch.crates-io]`. Keep the upstream
copyright notices and license when redistributing source or compiled packages.

## Local source changes

| File | Purpose |
| --- | --- |
| `src/sni.rs` | Pin the server certificate to the trusted pairing record and verify TLS transcript signatures instead of accepting them unconditionally. |
| `src/remote_pairing/rp_pairing_file.rs` | Remove debug output of the decoded pairing dictionary. |
| `src/services/wda.rs` | Preserve raw JSON hierarchy/active-app information, bound response size, include connection establishment in the timeout and surface WDA errors. |
| `src/xpc/mod.rs` | Keep host identity deterministic across connections to the same host environment. |
| `src/services/dvt/message.rs` | Expose message correlation metadata needed by the integration. |
| `src/services/dvt/remote_server.rs` | Expose correlated invocation submission and acknowledgement. |
| `src/services/dvt/xctest/mod.rs` | Bound WDA startup ownership, reject readiness from an earlier launch, and allow an explicitly supplied authenticated RSD stack to serve DVT/testmanagerd and readiness. An explicitly injected stack refuses DVT fallback outside that tunnel; the default USB fallback remains available. |
| `src/services/core_device/display_stream/client.rs` | Make the sender port optional to support the negotiated display-stream transport. |

These are local integration changes, not a claim of upstream acceptance. Review
them before changing the pinned dependency. The vendored package's generated
`Cargo.lock` is not used by the workspace and is excluded from publication.
Unmodified upstream test fixtures are retained with their original source.

The desktop's separate runner import uses public AFC/InstallationProxy APIs; no
personal runner, pairing record, signing certificate or key is included here.

# Architecture

## Components

| Component | Responsibility |
| --- | --- |
| `device-desktop` | Native GPUI Kit Windows workspace, preferences and independent phone window. |
| `device-api` | Versioned HTTP data types and error/operation contracts. |
| `device-client` | Shared authenticated Rust HTTP client and bounded video reader. |
| `device-daemon` | Device-scoped HTTP service, operation serialization, diagnostics and transport lifecycle. |
| `device-core` | Apple/usbmux, pairing-backed transport, device control and element providers. |
| `device-video` | HEVC decode pipeline and bounded latest-frame delivery to GUI consumers. |
| `device-diagnostics` | Layered host/service/USB evidence and explicitly enabled recovery. |
| `device-cli` | Command-line API client; package/binary name `rust-ios-device`. |
| `device-mcp` | Optional stdio MCP adapter using the shared API. |
| `device-gui` | Earlier egui interface retained in source; not the current Windows desktop. |

## Connections and identity

USB through Apple's installed multiplexer, Apple/usbmux network devices, direct
paired Wi-Fi and RSD are distinct paths. The selected device identity is checked;
physical USB presence alone is not evidence of a usable paired session.
An external local tunnel must exist before the RSD workflow can discover it.
Availability of video, touch and element providers is reported separately.

The GUI may own a scoped child daemon on an ephemeral loopback port, with a token
kept in memory. CLI and MCP clients use the same versioned HTTP API. Remote API
clients should use SSH forwarding. Protocol, authentication and error handling are
implemented in `device-api`, `device-client` and `device-daemon`; source is the
reference for exact routes and limits.

## Live screen and input

Encoded HEVC order is preserved for decoding. The preview is downscaled before
RGBA conversion; the GUI retains only the latest decoded frame instead of building
an image queue. Device source geometry and alignment padding are validated before
touch coordinates are mapped to physical pixels.

Video workers and input workers operate outside the UI thread. Continuous touch
uses ordered Start/Move/End/Cancel samples, one request in flight and the latest
pending position. A stationary contact sends a keepalive; abandoned contact has a
bounded daemon lease. Release/cancellation is retained through window teardown.
A completed drag is not replayed as a second animation.

Stream reconnection discards the previous stream. Device actions are never replayed
after an unknown result. Old frames block new input without being presented as
proof of a current device state. Geometry changes invalidate an in-progress gesture.

Statistics distinguish distinct frames painted by GPUI, `1000/FPS`, local decoded
frame age and API response time. None is a GPU presentation fence or a full
mouse-to-device latency measurement. There is no universal 30/60 FPS guarantee.

## Elements and runner installation

A configured XCTest/WebDriverAgent provider supplies element hierarchy and actions.
AX inspection can provide a read-only alternative where supported. References are
bounded and tied to observed state; activation must not reuse stale observations.
The provider's live readiness is distinct from compiled implementation support.

Runner import reads a bounded local IPA, checks ZIP/CMS/profile metadata and stages
an explicitly requested install over paired USB. It does not extract files on the
host or sign/build Xcode projects. iOS performs final installation signature checks.
See [runner import](RUNNER-IMPORT.md).

## Dependencies and publication

Apple protocol support is pinned to a locally patched `idevice` version. GPUI and
AccessKit compatibility patches are retained with licenses and provenance under
`vendor/`. See [third-party attribution](THIRD-PARTY.md).

No pairing record, developer private key or personal runner belongs in the source
or release package. Packaging uses an explicit file list; runtime recordings and
local evidence remain outside the public snapshot.

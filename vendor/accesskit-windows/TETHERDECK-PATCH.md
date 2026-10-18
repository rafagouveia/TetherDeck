# TetherDeck Windows accessibility teardown patch

## Provenance

- Package: `accesskit_windows` 0.34.0, copied from the Cargo registry source.
- Published package: <https://crates.io/crates/accesskit_windows/0.34.0>.
- Upstream: <https://github.com/AccessKit/accesskit>.
- Embedded revision: `c88605b96d04431f9c3c792464a0f2f253480e94`, path `platforms/windows`.
- Published archive SHA-256: `106c2b961215864d1c2e703ee63269c25c4e80a577ffb2c1017b9c17dcdf83a1`.
- License: MIT OR Apache-2.0. Source copyright notices are preserved. The published
  archive omits the referenced license files; `LICENSE-MIT` and `LICENSE-APACHE`
  were copied unchanged from the upstream root at the embedded revision.

## Local change

The application observed `ClientToScreen` failing with `ERROR_INVALID_WINDOW_HANDLE`
inside an accessible provider after closing a native window. Upstream
`client_top_left` unconditionally unwrapped that result, causing a panic through a
COM ABI and terminating the process.

The private helper and its `Context` wrapper now return `windows::core::Result`.
Failure becomes `UIA_E_ELEMENTNOTAVAILABLE`, propagated by the existing fallible
UI Automation implementations for bounding rectangles, hit testing, text range
hit testing and text bounding rectangles. Successful coordinates are unchanged;
no fallback coordinates, accessibility disabling or public API changes are added.

Only `src/util.rs`, `src/context.rs`, `src/node.rs`, and `src/text.rs` differ from
the published source. The workspace selects this crate through `[patch.crates-io]`.
This prevents this specific teardown panic; it does not change GPUI's native
window destruction ordering or guarantee that unrelated provider paths are free
from lifecycle races.

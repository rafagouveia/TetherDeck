# TetherDeck build compatibility patch

## Upstream source

- Package: `gpui-base` version `0.7.3`, copied from the existing Cargo registry source.
- Published package: <https://crates.io/crates/gpui-base/0.7.3>.
- Upstream repository: <https://github.com/longbridge/gpui-kit>.
- Archive VCS revision: `67265a36e260ffe03c5aaebdcd8d654dbf892eab`, path `crates/base`.
- Cargo archive SHA-256:
  `2e9955b6f4b048fd50ca15fc8e225d80bb00ba9ebc8fecdddde5ad26dbef9270`.
- License: Apache-2.0. The original `LICENSE-APACHE` and notices remain intact.

## Local change

`TextSelectionScopeId::new` in `src/text_selection.rs` now calls
`AtomicU64::fetch_update` instead of `AtomicU64::try_update`, which is unavailable
on the installed Rust 1.94.1 compiler.

Only the method name changed. Both memory orderings remain `Relaxed`, and the
closure remains `value.checked_add(1)`. The operation atomically updates the
counter and returns its previous value on success. Overflow still returns an
error and reaches the existing exhaustion panic. Concurrent retries and process
unique identifier allocation retain the same semantics.

The Rust standard library documents `fetch_update` as the earlier name for
`try_update`; its implementation delegates with the same orderings and closure:
<https://doc.rust-lang.org/src/core/sync/atomic.rs.html>.

No other upstream source or package metadata changed. The workspace override is
configured separately. No compiler/toolchain installation, compilation, tests or
UI operations were performed for this patch.

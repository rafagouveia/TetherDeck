# TetherDeck build compatibility patch

## Upstream source

- Package: `gpui-pre` version `0.3.9`, copied from the existing Cargo registry source.
- Published package: <https://crates.io/crates/gpui-pre/0.3.9>.
- Upstream repository: <https://github.com/zed-industries/zed>.
- Embedded upstream revision: `93d745668805029476e6d7fbb19652116fed51de`
  (`package.metadata.gpui-pre.zed-rev`).
- Cargo archive SHA-256:
  `34f61c1cfd36fc9c436af09308c963822ae27a45a84feb21a77ba5c9e051f1c0`.
- Upstream license: Apache-2.0. The original `LICENSE-APACHE` and notices remain intact.

## Local change

Rust 1.94.1 rejects the unstable `std::hint::cold_path` intrinsic used by
`TaskStatistics::add_yield_timing` and `TaskStatistics::add_runtime` in
`src/profiler.rs`.

The two unconditional calls now invoke a private empty helper annotated
`#[cold]`. This uses a stable optimization annotation and performs no runtime
work. Task timing calculations, comparisons, storage, profiler APIs and UI
behavior are unchanged. The compiler's optimization decisions need not be
identical to those produced by the intrinsic.

No other upstream source files or package metadata were changed.

## Scope

Four other `std::hint::cold_path` calls remain under the optional `profiler`
feature: two in `src/profiler.rs` and two in `src/profiler/actions.rs`.
Enabling that feature on Rust 1.94.1 requires a separate compatibility decision.

The workspace registry override is configured separately by TetherDeck.
No compiler/toolchain installation, compilation, test or UI operation was
performed as part of preparing this vendor patch.

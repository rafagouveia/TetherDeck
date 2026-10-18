# Third-party attribution

TetherDeck's original source and artwork use the repository's MIT license.
Dependencies retain their own licenses; that license does not replace theirs.

## Vendored source

| Package | Base version | Declared upstream license | Local provenance |
| --- | --- | --- | --- |
| idevice | 0.1.68 | MIT | [Patch and archive identity](../vendor/idevice/TETHERDECK-PATCH.md), [license](../vendor/idevice/LICENSE-MIT) |
| gpui-pre | 0.3.9 | Apache-2.0 | [Compatibility patch](../vendor/gpui-pre/TETHERDECK-PATCH.md), [license](../vendor/gpui-pre/LICENSE-APACHE) |
| gpui-base | 0.7.3 | Apache-2.0 | [Compatibility patch](../vendor/gpui-base/TETHERDECK-PATCH.md), [license](../vendor/gpui-base/LICENSE-APACHE) |
| accesskit_windows | 0.34.0 | MIT OR Apache-2.0 | [Teardown patch](../vendor/accesskit-windows/TETHERDECK-PATCH.md), [MIT](../vendor/accesskit-windows/LICENSE-MIT), [Apache-2.0](../vendor/accesskit-windows/LICENSE-APACHE) |

The Windows UI uses GPUI Kit and GPUI Component, with Lucide icons under ISC.
Packaged UI notices are in [THIRD-PARTY-GPUI.txt](THIRD-PARTY-GPUI.txt).
The earlier egui icon assets retain [their ISC notice](../crates/device-gui/assets/icons/LICENSE).
The current desktop's SVG icons retain the [Lucide and Feather notices](../crates/device-desktop/assets/icons/LICENSE-LUCIDE), with [local geometry notes](../crates/device-desktop/assets/icons/README.md).
System fonts are provided by the operating system; font files are not bundled.

## External tools and artifacts

FFmpeg is invoked as an external executable and is not included by TetherDeck's
Windows package script. Its applicable license depends on the chosen build and
configuration; consult [FFmpeg's legal information](https://ffmpeg.org/legal.html)
before separately redistributing a binary.

WebDriverAgent runners, Apple development certificates, provisioning profiles and
pairing material are supplied/prepared by each user or team. They are excluded
from public source and community packages. Apple device support is installed and
licensed separately by the user.

External protocol references and development utilities are not bundled merely
because they were used during development. Retain permission and provenance for
any code actually copied into a future patch; references are not a substitute for
license review.

## Release review

`Cargo.lock` records the resolved dependency versions. Review the actual target's
dependency graph and notices before releasing binaries; the table above covers
vendored sources and is not a complete software bill of materials. A pattern
search without GPL headers does not prove absence of licensing obligations.

An all-target metadata review found GPL-family alternatives in `r-efi` (MIT OR
Apache-2.0 OR LGPL-2.1-or-later) and `self_cell` (Apache-2.0 OR GPL-2.0-only).
The permissive MIT/Apache-2.0 alternatives can be selected for distribution with
their corresponding notices. Recheck the resolved graph when dependencies change;
this observation is not a complete target-specific license inventory.

# Desktop icons

Application icons use Lucide geometry copied from the published
`gpui-kit-assets` 0.7.3 package, with only the stroke width changed from 2 to
1.75 on a 24 px view box. `LICENSE-LUCIDE` was copied from that same package;
preserve its Lucide ISC and derived Feather MIT notices. Icon upstream:
<https://github.com/lucide-icons/lucide>.

`panel-left.svg` expresses the sidebar as a two-part panel. The four
`caption-*.svg` files are simple, locally drawn 10 px window-control glyphs.
They use the project's MIT license. They are rendered by GPUI and do not claim
to be Windows system-provided controls.

Use `src/icons.rs` to keep icon size and color consistent. Window glyphs are
added as button children so the component's default icon sizing does not enlarge
them. The main caption uses 46 × 36 px hit areas; the floating viewer retains
compact controls with the same symbols.

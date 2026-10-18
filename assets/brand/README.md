# TetherDeck identity

The original vector mark represents two connected device boundaries and their control link. It is not the Zed logo. The neutral background and soft blue accent match the application's One Dark inspired theme.

- `tetherdeck-mark.svg`: transparent symbol.
- `tetherdeck-app.svg`: application tile, source for PNG and Windows ICO exports.
- `tetherdeck-icon-256.png`, `tetherdeck-icon-64.png`, `tetherdeck.ico`: application assets.
- `tetherdeck-banner.png`: repository banner.

Colors: background `#282C33`, foreground `#DCE0E5`, accent `#74ADE8`, outline `#4B5263`. Original artwork is covered by the repository's MIT license. No font files are distributed.

## Regenerating exports

Edit the SVG sources, then run `python scripts/export-brand.py` from the repository root on Windows with CairoSVG and Pillow installed. The optional exporter uses the system Segoe UI font for the repository banner; fonts are not bundled. These Python tools are not runtime dependencies of TetherDeck.

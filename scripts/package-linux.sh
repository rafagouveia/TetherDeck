#!/bin/sh
set -eu
workspace=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binaries=${1:-"$workspace/target/release"}
destination="$workspace/outputs/linux"
mkdir -p "$destination"
for name in rust-ios-device rust-ios-device-daemon rust-ios-device-mcp; do
    test -f "$binaries/$name"
    install -m 755 "$binaries/$name" "$destination/$name"
done
install -m 755 "$binaries/rust-ios-device-gui" "$destination/tetherdeck"
install -m 644 "$workspace/docs/GETTING-STARTED.md" "$destination/COMECE-AQUI.md"
install -m 644 "$workspace/LICENSE" "$destination/LICENSE"
install -m 644 "$workspace/assets/brand/tetherdeck-icon-256.png" "$destination/tetherdeck.png"
cat > "$destination/README.txt" <<'EOF'
TetherDeck — Linux x86_64 — Preview 0.1

Run ./tetherdeck with all executables in the same directory.
Conectar provides separate USB and Wi-Fi workflows. Elementos, Tela remota,
Diagnostico and Integracoes separate the workspace responsibilities.

Native USB needs usbmuxd, USB/udev permissions and a trusted paired iPhone.
Direct Wi-Fi uses existing Lockdown and remote pairing records and native
Bonjour discovery. Live frames and gestures were exercised without USB on
a Windows host (about 1.43 fps); physical Linux validation is still pending.
RSD needs an already active local tunnel; RSD alone does not provide WDA
element activation. The live feed is not high-frame-rate HEVC video.

The GUI needs X11/Wayland, OpenGL and XKB/XCB runtime libraries, including
libxkbcommon-x11-0 on Ubuntu. WSL does not automatically receive Windows USB.
WDA needs a runner signed and provisioned by your own Apple team for your
device. No personal IPA, signing key or provisioning profile is bundled.

Free open source under MIT. Existing rust-ios-device-* and
RUST_IOS_DEVICE_* technical names remain compatible. Read COMECE-AQUI.md.
EOF
(cd "$destination" && sha256sum tetherdeck rust-ios-device rust-ios-device-daemon rust-ios-device-mcp README.txt COMECE-AQUI.md LICENSE tetherdeck.png > manifest.sha256)
tar -C "$destination" -czf "$workspace/outputs/tetherdeck-linux-x86_64.tar.gz" tetherdeck rust-ios-device rust-ios-device-daemon rust-ios-device-mcp README.txt COMECE-AQUI.md LICENSE tetherdeck.png manifest.sha256

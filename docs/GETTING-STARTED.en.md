# Getting started on Windows

[Português](GETTING-STARTED.md) · [Español](GETTING-STARTED.es.md)

## Requirements

- Windows x86_64 with a DirectX 11 capable graphics environment.
- Apple's Windows device drivers/service and a trusted, unlocked iPhone.
- An external FFmpeg executable with HEVC decoding for the live window.
- For element automation: a compatible signed WebDriverAgent XCTest runner,
  development provisioning for this device, and the device's required developer setup.

The desktop, daemon, CLI and MCP adapter are Rust programs; this application does
not require a Python interpreter. An externally managed RSD tunnel may have its
own setup requirements.

## Connect by USB

1. Keep all packaged executables together and open **TetherDeck.exe**.
2. Connect and unlock your iPhone. Accept Apple's trust request on the device if shown.
3. In the connection page choose USB, discover the device, then open its session.
4. Use diagnostics if the Apple service, usbmux, pairing or device identity is unavailable.

Physical USB presence, a running Apple service and successful paired device access
are different diagnostic layers. Recovery buttons require the relevant opt-in and
Windows permissions. A reported reboot requirement is not an automatic reboot.

## Connect over Wi-Fi

Apple/usbmux Wi-Fi, direct paired Wi-Fi and an existing RSD tunnel are separate
transports. Prepare trusted pairing over USB first where required; keep phone and
computer on a reachable local network. Choose the intended network transport and
use its discovery/identity result before opening the session.

If a workflow requires a remote pairing record, prepare that record through your
trusted pairing process; do not publish or share it. RSD discovery only consumes
an already running local tunnel. It does not create a tunnel or silently switch
transports after a possibly executed command.

Once the chosen network path is verified, supported Wi-Fi use does not require
the USB cable. Video/touch and element inspection have separate readiness states.

## Live screen

Choose **Open screen** after connecting. The independent transparent phone window
has a draggable header, four proportional resize corners, minimize/maximize/close,
and floating device controls. Press the mouse to touch, move while held to drag,
and release to lift the finger. Pausing, stale video and cancellation paths release
contact. A failed command is not replayed automatically.

The statistics panel separates GPUI paint FPS, a derived frame interval, local
frame age and API response time. It does not measure full mouse-to-device latency.

Set `TETHERDECK_FFMPEG` or put `ffmpeg.exe` beside the app. The remaining search
locations are `%ProgramFiles%\ffmpeg\ffmpeg.exe` and `PATH`. No FFmpeg binary is
bundled or downloaded by TetherDeck.

## Choose an inspection and automation provider

In **Elements**, select **Automatic**, **WebDriverAgent** or **iOS Accessibility**.
AX inspects through the paired tunnel, including Wi-Fi, without a WDA runner or
signing certificate. Its traversal may scroll the app and does not offer general
activation. WDA provides hierarchy and activation through fresh element references.
The response always identifies the provider actually used.

Wi-Fi WDA uses the development tunnel. First review that your runner binds both
HTTP and MJPEG to the internal tunnel address, then enable **Runner prepared for
isolated Wi-Fi** for that exact Bundle ID in session options. It is off by default;
changing the bundle or importing another IPA invalidates that review. A name or
version string alone is not sufficient. See [providers and requirements](ELEMENT-PROVIDERS.md).
AX remains available without that option.

## Elements and signing

Import a pre-signed WDA `.ipa` in session options, review its metadata, and click
install explicitly with the selected iPhone connected over paired USB. The Bundle
ID is filled after installation is confirmed; open a new compatible session to use it.
See [runner import](RUNNER-IMPORT.md) for package limits and certificate requirements.

A certificate alone is insufficient to build/sign the runner. TetherDeck does not
collect a signing key/password or compile Xcode projects on Windows. Each team
provides provisioning for its own devices; the author's certificate is not shared.

## Tools and preferences

The workspace exposes diagnostics, element inspection and integrations. MCP clients
use the same authenticated local API. Exported MCP configuration contains an API
token: treat it as a credential and do not attach it to an issue.

Use preferences to select system/English/Portuguese/Spanish language, appearance,
reduced motion and automatic discovery. Settings are stored locally, outside the
repository. Device and accessibility text is preserved in its original language.

For help, open a repository issue with redacted diagnostics and reproduction steps.
Do not attach pairing records, private keys, provisioning profiles or personal IPAs.

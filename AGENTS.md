# TetherDeck contributor and agent notes

## Scope

Windows is the active desktop target. The current application is `crates/device-desktop` (GPUI Kit); `device-gui` is the earlier interface. Keep device operations and decoding off the UI thread, and keep shared transport behavior in the core/client/daemon layers.

## Before editing

Read the relevant crate and `docs/ARCHITECTURE.md`. If a `.codegraph/` directory exists at the repository root, use CodeGraph before text search to locate or understand code. Do not create an index automatically. Without an index, use ordinary source search.

## Runtime contracts

- Preserve authentication, exact-device scoping and loopback API defaults.
- Never replay a device mutation after an uncertain result. Keep touch release, cancellation and timeout paths intact.
- Keep latest-frame/latest-position behavior; do not introduce unbounded media or input queues.
- Distinguish interface preference from whether a fresh frame permits input.
- Report protocol, hardware and verification limits honestly. A successful build does not prove behavior on a phone.

## Interface changes

Use shared theme roles and components. Update Portuguese, English and Spanish application copy together; never translate device content or machine protocol identifiers. Preserve accessible names, keyboard navigation, reduced motion and native window behavior.

## Validation and publication

Follow the current task's validation instructions. The standard compilation checks are documented in `CONTRIBUTING.md`. Do not claim checks you did not execute.

Never commit signing keys, certificates, provisioning profiles, pairing records, tokens, user settings, personal IPAs, recordings, scratch work or local build outputs. The publication script checks candidate paths and suspicious content without displaying matched values. Do not bypass it or force-push private local history into the public branch. Keep third-party licenses and exact patch provenance.

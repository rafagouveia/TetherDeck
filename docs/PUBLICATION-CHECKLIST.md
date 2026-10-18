# Public repository and release checklist

This checklist prepares a reviewable publication. It does not delete local data,
rewrite history, create a remote or push commits.

## Public source allowlist

- Root: manifests/lockfile, license, README translations, community docs and `.gitignore`.
- `.cargo/`: documented shared build/advisory configuration, never credentials.
- `.github/`: issue/PR templates and Windows compilation/hygiene workflow.
- `crates/`: maintained Rust source, build scripts and public application assets.
- `assets/`: original product artwork and its provenance notes.
- `docs/`: user/developer guides and third-party license notices.
- `scripts/`: generic setup/package/recovery/hygiene scripts with placeholders.
- `vendor/`: necessary pinned source, original licenses and local patch provenance.

Review new paths individually; a directory's presence in this list is not approval
for arbitrary archives, secrets or generated data inside it.

## Keep private or reproducible

| Path/type | Treatment |
| --- | --- |
| `work/` | Private scratch work, diagnostics and potentially sensitive local setup. Archive outside the public repository if needed. |
| `.zsign_cache/`, pairing/signing caches | Keep private; may contain device authorization or signing material. |
| `target/` and nested build targets | Reproducible build output; exclude from Git. |
| `outputs/` | Local deliverables/evidence; ignored. Publish only a separately reviewed release allowlist. |
| Keys, `.p12`, `.pfx`, `.p8`, `.pem`, profiles, personal `.ipa` | Never include in source or community releases. |
| Logs, captures, recordings, crash/network traces | Keep local; redact and select an explicit sample before sharing. |
| `.env`, MCP exports and local preferences | Keep outside the public snapshot. |

## Before publication

1. Review `git status --short` and the complete source file list, including ignored
   files that might already be tracked. Do not use blanket staging as a substitute.
2. Run `scripts/check-publication.ps1`; its findings contain paths/types, not matched values.
3. Inspect historical content and author/committer identity before publishing existing
   history. `.gitignore` does not erase a secret or email from an earlier commit.
4. Check screenshots, docs and examples for personal device identifiers, usernames,
   addresses, pairing material and credentials. Use placeholders or documentation addresses.
5. Preserve vendor licenses, pinned versions, archive hashes and patch notes. Review
   transitive dependency and external tool licensing for the actual release artifact.
6. Run the documented Windows compilation checks. Record what was checked; do not
   describe a source-only check as physical device or performance validation.
7. Package using the explicit script allowlist. Review the archive's file list and
   hashes before attaching it to a release. External FFmpeg is not bundled.
8. Review the destination and visibility before the maintainer performs the final push.

Do not delete pairing or signing data merely to prepare a repository. A private
archive outside the publication tree preserves local development access without
making it public. Any actual cleanup/move must verify exact absolute target paths.

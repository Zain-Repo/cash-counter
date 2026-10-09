# Release process

The first public version is `0.0.1`. Release tags use `vX.Y.Z` and must match
`Cargo.toml`. The product and installer are named Cash Ledger; the Rust package and
repository are named `cash-counter`.

## Publish a version

1. Update the version in `Cargo.toml` and refresh the project entry in Cargo.lock
   with `cargo check`. Preserve third-party dependency versions.
2. Add `releases/vX.Y.Z.md` describing implemented changes and known limitations.
3. Run formatting, checks, Clippy and tests, then commit and push to `main`.
4. The Validate, build and release workflow runs version validation and checks,
   then installer packaging, then the reusable Release workflow sequentially in
   the same run. No manual tag push or separate release run is required.
   Pull requests run checks and packaging without publishing.

After checks and packaging succeed on main, the release job reads the manifest
version, requires matching release notes, and verifies SHA-256 checksums of the
installer artifact from that run. It creates `vX.Y.Z` at the tested commit,
uploads assets to a draft, and publishes it as the latest release. An existing
tag must point to the same tested commit; published tags are never moved.
Rerunning the same published commit leaves its release unchanged. A new commit
for distribution needs a new version and release notes.

Only the release job receives `contents: write`; checks and packaging remain
read-only. Checkout credentials are not persisted. Fork PRs use the ordinary
`pull_request` event. Runs are serialized per branch and are not automatically
cancelled while publishing.

If a build fails, inspect the failed job logs and fix the cause. Rerun transient
failures. Do not move an existing published version tag; publish a new patch
version for source fixes. A failed draft upload can be retried before publication.

## Toolchain and action maintenance

Rust is pinned to the previously validated 1.96.0 toolchain in
`rust-toolchain.toml`; toolchain updates should be validated as explicit changes.
Windows jobs use the current Windows Server 2025 image with the C++ toolchain,
Windows SDK and Inno Setup installed. Framework packages stay pinned together.

Action versions were checked against official GitHub releases on October 8,
2026: checkout 7.0.1, cache 6.1.0, upload-artifact 7.0.2 and download-artifact
8.0.2. Workflows pin their full commit SHAs, use the Node 24 action runtime, and
Dependabot checks the Actions ecosystem weekly. Review and merge updates after
CI passes; there is no unreviewed automatic merge.

Research: [checkout releases](https://github.com/actions/checkout/releases),
[cache releases](https://github.com/actions/cache/releases),
[upload-artifact releases](https://github.com/actions/upload-artifact/releases),
[download-artifact releases](https://github.com/actions/download-artifact/releases),
[Windows runner software](https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md),
[GitHub CLI release creation](https://cli.github.com/manual/gh_release_create),
and [Dependabot configuration](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference).

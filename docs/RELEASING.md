# Apollo Releasing

> [!IMPORTANT]
> All of the documentation here are written by LLMs.

Apollo distributes a per-user Windows x64 NSIS installer and a portable ZIP. Both contain the same application, pinned FFmpeg and FFprobe pair, and required redistribution material. The release process currently builds a draft GitHub Release from an explicitly created tag. Automatic release detection and publication are not active yet.

## Prepare a release

1. Choose the intended stable version (`X.Y.Z`). Use `just set-version X.Y.Z`, substituting that version. `Cargo.toml` owns the version; Cargo updates its root entry in `Cargo.lock`. The executable's resource version comes from `build.rs` and `assets/apollo.rc`; the installer uses the same Cargo version. Each numeric component must fit a Windows unsigned 16-bit version field (0 to 65535). The version command does not edit notes or create a tag.
2. Add the dated release section to `CHANGELOG.md` and update the version comparison links, including Unreleased. Keep the changelog technical. Write the canonical GitHub body in `release-notes/vX.Y.Z.md` following [the note-writing guide](../release-notes/README.md).
3. Run `just check-release --tag vX.Y.Z --require-changelog` with the intended tag, then `just verify`. Review the final diff. Candidate validation requires an exact tag, one valid dated changelog heading, the matching comparison URL and the versioned note with both download links and at least one change. Normal CI checks only version metadata; ordinary changes need not create a release entry.
4. Submit the release preparation PR. Wait for `CI Gate` to succeed on the merged commit's main-push run before creating a release tag. Only the repository owner or an explicitly authorized release operator may create tags or publish releases.

The version update is transactional: if Cargo fails or modifies the lockfile beyond the root version, both version files are restored. A focused local release-policy fixture check is available with `pwsh -NoProfile -File tests/scripts/ReleasePolicy.Tests.ps1`. It is independent of CI and `just verify`. Run it after `just build-release` on Windows with NSIS 3.11 installed to include the artifact and staging failure cases; without the built executable or compiler the unavailable fixture cases are reported as skipped. Real installer and media acceptance use the Windows smoke path.

## Detect release candidates (read-only)

The candidate detector is available for inspection before the automatic workflow is activated:

```powershell
just detect-release-candidate SHA TommyMoonn/apollo
```

Replace `SHA` with the full lowercase commit SHA of a clean checkout with complete Git history. The checkout origin must identify the supplied GitHub repository. The detector validates current version metadata and compares the Cargo package version against the commit's first parent, including on merge commits. An unchanged stable version is a noncandidate; a downgrade, invalid version or missing history is an error. An increase must also satisfy the full changelog and release-note checks.

For an increase, authenticated GitHub CLI access is required. Detection checks the exact remote tag (including annotated tags), exact release identity and all pages of release records visible to that credential. A matching tag, draft or published release can be recognized for retry; missing or ambiguous identities, conflicting targets and failed queries stop detection. A published release is not modified. GitHub can hide drafts from credentials without sufficient access; a candidate result is not proof that no hidden draft exists. The command emits JSON with `candidate`, `version`, `sha` and `reason`, and writes the same fields to `GITHUB_OUTPUT` when configured. It does not authorize publication or prove CI success. The future publisher must recheck remote state and artifact integrity before any write.

The active tag-based workflow below remains unchanged. Do not create a tag or release merely to test detection.

## Build and inspect packages

Run the `Windows Installer Build` workflow manually for a nonpublishing build of the selected branch or tag revision. It reuses successful main-push `CI Gate` evidence for that exact SHA when available; otherwise it runs the ordinary source checks once. Manual and tagged release builds call the same `windows-package.yml` workflow, which builds the Windows release once, stages and accepts it, uploads the four fixed files, and independently verifies the downloaded artifact by ID. The workflow summary identifies the version, SHA, artifact ID, run ID and attempt. Acceptance logs are uploaded separately; neither the logs nor provenance are published as release assets.

For a local build, use the local Windows x64 development environment with NSIS 3.11:

```powershell
just build-release
just stage-release
just verify-release --local --expected-version 0.7.0
just smoke-installer --local --expected-version 0.7.0
```

The commands build the locked release executable, stage and independently verify the artifacts, and run native acceptance without rebuilding the application. The smoke command requires a clean Windows user or VM and an official published installer with its checksum. Manual builds at the current published version test a same-version reinstall; tagged releases require an older version. See [Testing](TESTING.md#windows-artifact-acceptance) for isolation and GPU options. Substitute the current Cargo version in the verification command. The three public files in `artifacts/windows/` are:

- `Apollo-Setup-x64.exe`
- `Apollo-windows-x64.zip`
- `SHA256SUMS.txt`

The same directory contains `release-provenance.json`, an internal schema-versioned record of the candidate identity, selected Rust and NSIS versions, pinned media archive, and asset and payload digests and sizes. It is transferred with CI artifacts but never published. Without explicit candidate identity, staging records a null SHA even for a clean local tree. For a clean candidate checkout, pass `--expected-sha <full-sha> --expected-version X.Y.Z` to both `just stage-release` and `just verify-release`. Use `--artifacts-dir <path>` to verify a downloaded copy. The verifier reads trusted metadata, notices and the media manifest from that checkout, and needs neither the original build output nor media cache. Media redistribution bytes are checked against the recorded digests from the validated archive. Provenance is not a signature and does not independently prove the origin of a build. Windows resource inspection requires Windows; runtime installer acceptance is separate.

Use [Testing](TESTING.md) for checksum, installed and portable acceptance, CPU encoding and manual NVIDIA checks. The [media-tool acceptance record](../distribution/media-tools-acceptance.md) describes the pinned binaries and recorded hardware evidence. A previous release's acceptance is not evidence for newly built artifacts. See [repository scripts](../scripts/README.md) for staging options and prerequisites.

## Publish using the current tag workflow

The following procedure remains active until the automatic release workflow replaces it:

1. With authorization, create and push the exact `vX.Y.Z` tag on the intended green main commit. The `Release` workflow verifies that the tag belongs to main and reuses the successful push CI run for that commit. It validates the full candidate metadata, builds Windows artifacts, runs release smoke acceptance, and independently verifies the downloaded artifact set before creating or refreshing a draft.
2. Review the draft and verify its three assets. A newly created draft uses the canonical note file; when refreshing an existing draft, the workflow leaves its body unchanged. Confirm the draft body matches the tracked note before publishing. The workflow refuses to replace published release assets.
3. Complete the manual hardware acceptance in [Testing](TESTING.md), using the actual staged artifacts. Resolve any failed acceptance before publication. Publish the draft only after the required checks and review succeed.

The current workflow does not yet provide automatic candidate detection or retry-safe asset reconciliation. Do not overwrite conflicting published artifacts or move a release tag to recover from a failed build. Investigate the failed run and coordinate recovery with the owner.

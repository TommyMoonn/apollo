# Apollo Releasing

> [!IMPORTANT]
> All of the documentation here are written by LLMs.

Apollo distributes a per-user Windows x64 NSIS installer and a portable ZIP. Both contain the same application, pinned FFmpeg and FFprobe pair, and required redistribution material. The release process currently builds a draft GitHub Release from an explicitly created tag. Automatic release detection and publication are not active yet.

## Prepare a release

1. Choose the intended stable version (`X.Y.Z`). Use `just set-version X.Y.Z`, substituting that version. `Cargo.toml` owns the version; Cargo updates its root entry in `Cargo.lock`. The executable's resource version comes from `build.rs` and `assets/apollo.rc`; the installer uses the same Cargo version. Each numeric component must fit a Windows unsigned 16-bit version field (0 to 65535). The version command does not edit notes or create a tag.
2. Add the dated release section to `CHANGELOG.md` and update the version comparison links, including Unreleased. Keep the changelog technical. Write the canonical GitHub body in `release-notes/vX.Y.Z.md` following [the note-writing guide](../release-notes/README.md).
3. Run `just check-release --tag vX.Y.Z --require-changelog` with the intended tag, then `just verify`. Review the final diff. Candidate validation requires an exact tag, one valid dated changelog heading, the matching comparison URL and the versioned note with both download links and at least one change. Normal CI checks only version metadata; ordinary changes need not create a release entry.
4. Submit the release preparation PR. Wait for `CI Gate` to succeed on the merged commit's main-push run before creating a release tag. Only the repository owner or an explicitly authorized release operator may create tags or publish releases.

The version update is transactional: if Cargo fails or modifies the lockfile beyond the root version, both version files are restored. A focused local release-policy fixture check is available with `pwsh -NoProfile -File tests/scripts/ReleasePolicy.Tests.ps1`. It is independent of CI and `just verify`.

## Build and inspect packages

Run the `Windows Installer Build` workflow manually for a nonpublishing build, or use the local Windows x64 development environment with NSIS 3.11:

```powershell
just release-stage
```

The local command builds the locked release executable and stages the following files in `artifacts/windows/`:

- `Apollo-Setup-x64.exe`
- `Apollo-windows-x64.zip`
- `SHA256SUMS.txt`

Use [Testing](TESTING.md) for checksum, installed and portable acceptance, CPU encoding and manual NVIDIA checks. The [media-tool acceptance record](../distribution/media-tools-acceptance.md) describes the pinned binaries and recorded hardware evidence. A previous release's acceptance is not evidence for newly built artifacts. See [repository scripts](../scripts/README.md) for staging options and prerequisites.

## Publish using the current tag workflow

The following procedure remains active until the automatic release workflow replaces it:

1. With authorization, create and push the exact `vX.Y.Z` tag on the intended green main commit. The `Release` workflow verifies that the tag belongs to main and reuses the successful push CI run for that commit. It validates the full candidate metadata, builds Windows artifacts, runs release smoke acceptance, and verifies the downloaded checksums before creating or refreshing a draft.
2. Review the draft and verify its three assets. A newly created draft uses the canonical note file; when refreshing an existing draft, the workflow leaves its body unchanged. Confirm the draft body matches the tracked note before publishing. The workflow refuses to replace published release assets.
3. Complete the manual hardware acceptance in [Testing](TESTING.md), using the actual staged artifacts. Resolve any failed acceptance before publication. Publish the draft only after the required checks and review succeed.

The current workflow does not yet provide automatic candidate detection or retry-safe asset reconciliation. Do not overwrite conflicting published artifacts or move a release tag to recover from a failed build. Investigate the failed run and coordinate recovery with the owner.

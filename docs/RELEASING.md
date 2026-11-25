# Apollo Releasing

> [!IMPORTANT]
> All of the documentation here are written by LLMs.

Apollo distributes a per-user Windows x64 NSIS installer and a portable ZIP. Both contain the same application, pinned FFmpeg and FFprobe pair, and required redistribution material. A stable version increase merged to `main` starts release processing after its push-triggered CI run succeeds. The release workflow creates the tag and publishes verified packages; operators do not push release tags.

## Prepare a release

1. Choose the intended stable version (`X.Y.Z`). Use `just set-version X.Y.Z`, substituting that version. `Cargo.toml` owns the version; Cargo updates its root entry in `Cargo.lock`. The executable's resource version comes from `build.rs` and `assets/apollo.rc`; the installer uses the same Cargo version. Each numeric component must fit a Windows unsigned 16-bit version field (0 to 65535). The version command does not edit notes or create a tag.
2. Add the dated release section to `CHANGELOG.md` and update the version comparison links, including Unreleased. Keep the changelog technical. Write the canonical GitHub body in `release-notes/vX.Y.Z.md` following [the note-writing guide](../release-notes/README.md).
3. Run `just check-release --tag vX.Y.Z --require-changelog` with the intended tag, then `just verify`. Review the final diff. Candidate validation requires an exact tag, one valid dated changelog heading, the matching comparison URL and the versioned note with both download links and at least one change. Normal CI checks only version metadata; ordinary changes need not create a release entry.
4. Submit the release preparation PR and review the candidate before merging. Complete the applicable manual hardware acceptance in [Testing](TESTING.md) on a nonpublishing build before authorizing the merge. A version increase must land in a main commit whose first parent has the previous version. After merging, monitor the main-push `CI` and the `Release` workflow; publication is automatic, not a separate manual approval step.

The version update is transactional: if Cargo fails or modifies the lockfile beyond the root version, both version files are restored. A focused local release-policy fixture check is available with `pwsh -NoProfile -File tests/scripts/ReleasePolicy.Tests.ps1`. It is independent of CI and `just verify`. Run it after `just build-release` on Windows with NSIS 3.11 installed to include the artifact and staging failure cases; without the built executable or compiler the unavailable fixture cases are reported as skipped. Real installer and media acceptance use the Windows smoke path.

## Detect release candidates (read-only)

Run the candidate detector locally to inspect a version transition without writing to GitHub:

```powershell
just detect-release-candidate SHA TommyMoonn/apollo
```

Replace `SHA` with the full lowercase commit SHA of a clean checkout with complete Git history. The checkout origin must identify the supplied GitHub repository. The detector validates current version metadata and compares the Cargo package version against the commit's first parent, including on merge commits. An unchanged stable version is a noncandidate; a downgrade, invalid version or missing history is an error. An increase must also satisfy the full changelog and release-note checks.

For an increase, authenticated GitHub CLI access is required. Detection checks the exact remote tag (including annotated tags), exact release identity and all pages of release records visible to that credential. A matching tag, draft or published release can be recognized for retry; missing or ambiguous identities, conflicting targets and failed queries stop detection. A published release is not modified. GitHub can hide drafts from credentials without sufficient access; a candidate result is not proof that no hidden draft exists. The command emits JSON with `candidate`, `version`, `sha` and `reason`, and writes the same fields to `GITHUB_OUTPUT` when configured. It does not authorize publication or prove CI success. The publisher rechecks remote state and artifact integrity before any write.

Do not create a tag or release to test detection.

## Build and inspect packages

Run the `Windows Installer Build` workflow manually for a nonpublishing build of the selected branch or tag revision. It reuses successful main-push `CI Gate` evidence for that exact SHA when available; otherwise it runs the ordinary source checks once. Manual and automatic release builds use `windows-package.yml` to build the Windows release once, stage and accept it, and upload the four fixed files. Both call `windows-verify.yml` to independently verify the downloaded artifact by ID. The workflow summary identifies the version, SHA, artifact ID, run ID and attempt. Acceptance logs are uploaded separately; neither the logs nor provenance are published as release assets.

For a local build, use the local Windows x64 development environment with NSIS 3.11:

```powershell
just build-release
just stage-release
just verify-release --local --expected-version 0.7.0
just smoke-installer --local --expected-version 0.7.0
```

The commands build the locked release executable, stage and independently verify the artifacts, and run native acceptance without rebuilding the application. The smoke command requires a clean Windows user or VM and an official published installer with its checksum. Manual builds at the current published version test a same-version reinstall; release candidates require an older version. See [Testing](TESTING.md#windows-artifact-acceptance) for isolation and GPU options. Substitute the current Cargo version in the verification command. The three public files in `artifacts/windows/` are:

- `Apollo-Setup-x64.exe`
- `Apollo-windows-x64.zip`
- `SHA256SUMS.txt`

The same directory contains `release-provenance.json`, an internal schema-versioned record of the candidate identity, selected Rust and NSIS versions, pinned media archive, and asset and payload digests and sizes. It is transferred with CI artifacts but never published. Without explicit candidate identity, staging records a null SHA even for a clean local tree. For a clean candidate checkout, pass `--expected-sha <full-sha> --expected-version X.Y.Z` to both `just stage-release` and `just verify-release`. Use `--artifacts-dir <path>` to verify a downloaded copy. The verifier reads trusted metadata, notices and the media manifest from that checkout, and needs neither the original build output nor media cache. Media redistribution bytes are checked against the recorded digests from the validated archive. Provenance is not a signature and does not independently prove the origin of a build. Windows resource inspection requires Windows; runtime installer acceptance is separate.

Use [Testing](TESTING.md) for checksum, installed and portable acceptance, all five queued media operations, mixed batches, CPU encoding and manual NVIDIA checks. For queue lifecycle changes, complete the native interface and graceful-close checks on the nonpublishing Windows build before approving release preparation. The [media-tool acceptance record](../distribution/media-tools-acceptance.md) describes the pinned binaries and recorded hardware evidence. A previous release's acceptance is not evidence for newly built artifacts. See [repository scripts](../scripts/README.md) for staging options and prerequisites.

## Automatic publication

`Release` listens for completed `CI` runs on `main`. Only a successful push from this repository qualifies. Detection checks the specific CI run and attempt, its successful `CI Gate`, and the candidate's ancestry on `origin/main`. It checks out the triggering SHA, not the latest main revision. An unchanged version skips the remaining jobs; a failed detection or conflict stops the run.

A candidate follows four stages:

1. Detect and validate the version increase and canonical release metadata.
2. Build and accept Windows packages through `windows-package.yml`. The green CI run already covers ordinary source checks; the release build runs once. Native acceptance covers portable use, installation, upgrade and uninstall.
3. Independently verify the four files through `windows-verify.yml`. The immutable artifact ID must belong to the accepted packaging run and match its name, version, SHA and attempt. Verification emits the SHA-256 of the checked provenance file.
4. Publish in a separate Windows job. It checks out the candidate again, downloads that same artifact ID, compares the provenance digest, and invokes the publisher, which checks every artifact again before writing. Only the three public files are uploaded to GitHub Releases.

The source run and artifact identity appear in the job summaries. Review native and applicable hardware acceptance before merging the release preparation PR. A pre-merge build is separate evidence; automatic publication builds the green main SHA and does not wait for a manual GPU check. Build artifacts and separate acceptance logs are retained for 30 days, subject to the repository retention policy. A manual `Windows Installer Build` never publishes. The required `CI Gate` remains independent of the release workflow.

### Repository permissions

Require `CI Gate` on the main branch ruleset. Protect the main history against force pushes and restrict release tag creation, updates and deletion for `v*`. The release workflow defaults to `contents: read`; detection and independent artifact verification also require `actions: read`. Only the publication job requests `contents: write`, using the repository's `GITHUB_TOKEN`. The token must be permitted to create the release tag and publish the release. Verify the default branch is `main` and the `CI` workflow and `Release` workflow are enabled. If a tag ruleset blocks that identity, stop and have the owner authorize an appropriate narrowly scoped automation identity under the existing protection policy. Do not disable tag protection or introduce a broad personal access token as a workaround. Verify repository settings in GitHub before the first authorized release; workflow code alone cannot establish those settings.

## Publication recovery

The publication entry point is reserved for the authorized workflow job; there is no local publication recipe. Publication is serialized across versions in a noncancelling concurrency group. GitHub can replace a pending job with a newer one; a cancelled, superseded or interrupted candidate is not evidence of publication. The publisher rechecks release ordering and remote identity after obtaining the slot.

Publication revalidates the candidate and the original artifact bytes before writing. It creates a missing tag at the candidate SHA, resolves the matching draft by its exact tag and release ID, and uploads only missing assets. Existing assets must match the expected names, sizes and SHA-256 digests. If GitHub does not supply a digest, the publisher downloads and hashes the asset. It requires the exact tracked note and `Apollo vX.Y.Z` title, verifies the complete draft before publication, and verifies the published release again afterward. The publisher rejects a new candidate if a higher stable version has already been published. A matching already-published release is a no-op, even if a newer version exists.

After an interrupted run, preserve the original candidate SHA and verified artifact set. Rerun the failed publication job (or failed jobs downstream of a successful packaging job) in the original eligible `Release` run. Check that the build artifact ID, run/attempt and provenance digest still match. Do not rerun all jobs when publication may have partially succeeded; rebuilding can produce different digests. If packaging failed before a verified artifact existed, rerun the failed packaging and dependent jobs on the original eligible revision. If a later candidate has published, the publisher rejects an older new release; an exact published replay is still a no-op. A matching existing tag, draft or uploaded asset is reused. A lost publication response can be resolved by replaying the candidate and verifying the already-published release.

Stop and request owner review if the tag, release identity, title, body or asset bytes conflict, if a remote lookup is inconclusive, or if the original verified artifacts are unavailable. A failed upload can leave an empty `starter` asset; the publisher treats it as a conflict rather than deleting or replacing it. Review the failure and the recorded digests before any separately authorized repair. Never move a tag or overwrite a published release to recover a run. The publisher does not delete remote state or edit conflicting drafts. The concurrency group does not prevent external owner changes, so repository tag protections still matter. If the original artifact expired or the original workflow cannot be rerun, stop for owner review. Do not substitute a manual build or trigger publication by pushing a tag.

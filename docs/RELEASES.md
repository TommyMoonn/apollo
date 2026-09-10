# Releasing Apollo

Apollo releases are prepared on `main`, built on GitHub Actions for Windows x64, and created as draft GitHub Releases for manual review and publication.

The release tag identifies the exact source revision used to build the attached installer and portable archive.

## Release artifacts

A successful Windows release stage produces exactly:

```text
Apollo-Setup-x64.exe
Apollo-windows-x64.zip
SHA256SUMS.txt
```

`Apollo-Setup-x64.exe` is the primary distribution. It installs Apollo for the current Windows user under `%LOCALAPPDATA%\Programs\Apollo`, creates an Apollo Start Menu shortcut, registers an Installed Apps entry with an uninstaller, and requires no elevation.

`Apollo-windows-x64.zip` is the portable alternative. It does not register or install Apollo. The archive contains exactly:

```text
Apollo.exe
ffmpeg.exe
ffprobe.exe
LICENSE
THIRD_PARTY_NOTICES.md
licenses/ffmpeg/LICENSE
licenses/ffmpeg/README.txt
licenses/ffmpeg/media-tools.json
```

Starting with v0.5.0, both distribution forms carry this same payload; the installer also owns `Uninstall.exe`. The pair is self-contained for Apollo's CPU media workflows, with no FFmpeg/FFprobe PATH requirement. Older published packages are not changed by this work.

`SHA256SUMS.txt` contains lowercase SHA-256 entries for the installer and portable ZIP, in that order.

## Prepare the release

Run release preparation from a clean Windows clone with Git, PowerShell 7, and the Rust toolchain selected by `rust-toolchain.toml` available.

Set the intended version. The script updates `Cargo.toml` and the root Apollo entry in `Cargo.lock`, validates the result with locked Cargo metadata, and rolls both files back if it fails.

```powershell
$version = '0.5.0'
$tag = "v$version"

.\scripts\set-version.ps1 $version
```

Finalize `CHANGELOG.md` before tagging:

- keep `## [Unreleased]` at the top;
- add a dated `## [<version>] - YYYY-MM-DD` release section;
- move or curate the shipped changes into that release section;
- add the matching `[<version>]` comparison link and make sure it references `v<version>`.

Validate the version, intended tag, and dated changelog entry together:

```powershell
.\scripts\check-release.ps1 --tag $tag --release-changelog
```

Run the complete local quality gate:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
pwsh -NoProfile -File .\tests\scripts\run.ps1
```

Commit the release preparation and get that exact commit onto `main`. Push it and resolve any failures reported by the `CI` workflow before tagging. The tag workflow requires the push-triggered `ci.yml` run for the exact tagged revision to succeed, then runs PowerShell tooling checks again in the Windows build job. It does not dispatch a second CI run for the tag.

Before creating the tag, verify the working tree is clean and local `HEAD` matches `origin/main`:

```powershell
git fetch origin main
git status --short

git rev-parse HEAD
git rev-parse origin/main
```

`git status --short` must produce no output, and the two commit IDs must be identical.

## Optional pre-tag Windows artifact check

The manually dispatched `Windows Installer Build` workflow runs formatting, Clippy and Rust tests before its Windows build job, which also runs PowerShell tooling checks. It uses the same staging owner as tagged releases and uploads one Actions artifact named `Apollo-windows-x64` containing the three staged release files. Both workflows run the installed/portable CPU and failure-path acceptance below before uploading; neither requires NVIDIA hardware.

That Actions artifact is temporary build output with short retention. It is **not** a GitHub Release and does not publish anything to users.

Use it when you want to inspect the Windows installer and portable package before creating a tag.

## Tag the release

Create the version tag only after the release commit is on `main` and the normal `CI` run is green:

```powershell
git tag $tag
git push origin $tag
```

Pushing `v*` starts the `Release` workflow. Before building, the workflow verifies that:

- the tag exactly matches the Cargo package version;
- `CHANGELOG.md` has the dated release section and matching version link;
- the tagged commit is contained in `main`;
- the existing push CI run for the exact tagged revision succeeds, and the Windows job's PowerShell tooling checks pass.

The verification job checks `main` ancestry and waits for that CI run. The Windows job then validates release metadata, builds the GUI x64 release, stages the installer, portable ZIP and checksum file, and checks the actual payloads. The final job verifies staged checksums and creates or refreshes a **draft** GitHub Release titled `Apollo <tag>`.

## Review the draft release

Do not publish the draft until the attached files have been checked.

### Verify checksums

Download these three assets from the draft release into one directory:

```text
Apollo-Setup-x64.exe
Apollo-windows-x64.zip
SHA256SUMS.txt
```

Then run:

```powershell
$lines = @(Get-Content .\SHA256SUMS.txt)
if ($lines.Count -ne 2) {
    throw "SHA256SUMS.txt must contain exactly two entries."
}

$expected = @{}
$lines | ForEach-Object {
    if ($_ -notmatch '^([0-9a-f]{64})  (.+)$') {
        throw "Invalid checksum line: $_"
    }
    $expected[$Matches[2]] = $Matches[1]
}

foreach ($name in 'Apollo-Setup-x64.exe', 'Apollo-windows-x64.zip') {
    if (-not $expected.ContainsKey($name)) {
        throw "Missing checksum for $name"
    }

    $actual = (Get-FileHash -LiteralPath $name -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $expected[$name]) {
        throw "SHA-256 mismatch for $name"
    }
}
```

The checksum file must contain exactly one entry for each of those two artifacts.

### Verify the installer

Run `Apollo-Setup-x64.exe` as the current user and verify:

- Apollo installs under `%LOCALAPPDATA%\Programs\Apollo`;
- the install contains the complete payload listed above and `Uninstall.exe`;
- an Apollo shortcut appears in the current user's Start Menu;
- Apollo appears in Windows Installed Apps with a working uninstaller;
- Apollo launches without an attached console window;
- uninstall removes Apollo's installed files and registration without removing unrelated user files.

For an upgrade check, install the new draft over an existing Apollo per-user installation and confirm the same Apollo registration is updated rather than creating a second application entry.

### Verify the portable ZIP

Expand `Apollo-windows-x64.zip` into an empty directory and confirm that it contains exactly the payload listed under Release artifacts, including the complete media-tool pair and `licenses/ffmpeg/` material.

The portable form must run without creating an Apollo Installed Apps entry or Start Menu shortcut.

## Pinned media distribution and offline staging

[`distribution/media-tools.json`](../distribution/media-tools.json) is the only binary-selection owner. The current Windows x64 pin is FFmpeg source 7.0.2, Gyan build `7.0.2-essentials_build-www.gyan.dev`, archive `ffmpeg-7.0.2-essentials_build.zip`, SHA-256 `d5308d30872b2739cf53169df61faba8639d39a19b20b91e611c177ef676f64c`. Use its versioned URL, never a distributor's rolling latest URL or an arbitrary PATH installation.

`stage-windows-release.ps1` stages the pair once via `stage-media-tools.ps1` and supplies those exact bytes to both packaging consumers. Standalone `build-windows-installer.ps1` uses that same staging owner. The default cache is `.scratch/media-tools-cache`; workflow cache keys include the manifest hash, and staging still verifies SHA-256 after cache restore.

From the repository root, with NSIS 3.11 available:

```powershell
cargo build --locked --release --target x86_64-pc-windows-msvc
pwsh -NoProfile -File .\scripts\stage-windows-release.ps1 --media-cache .scratch/media-tools-cache
# Repeat entirely from a valid cache, including installer compilation:
pwsh -NoProfile -File .\scripts\stage-windows-release.ps1 --offline --media-cache .scratch/media-tools-cache
```

Use `--makensis <path>` for an explicit compiler and `--output <directory>` for a different artifact destination. `--media-cache` and `--offline` also apply to the standalone installer builder. These release-script relative paths resolve from the repository root.

Offline mode never downloads. A cache miss or checksum mismatch fails before extraction; identify the exact corrupt archive and replace it from the pinned URL or use a fresh cache. Do not disable hash checks or change the pin to accept a corrupt download. Media staging validates both `-version` outputs and required encoders without invoking NVENC. Failure leaves existing published release output intact. Individual media staging requires a new destination; release staging can atomically replace its owned output directory.

## Self-contained media acceptance

Run against the **exact artifacts being reviewed**, with the checkout and built `Apollo.exe` matching their revision. The script checks artifact sums, compares every payload byte to the build and a freshly verified offline archive extraction, and verifies installed metadata. It refuses to touch an existing Apollo registration or shortcut, so use a clean Windows user or VM if Apollo is already installed.

```powershell
$harness = @(cargo test --locked --test bundled_media_tools --no-run --message-format=json |
    ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'bundled_media_tools' -and $_.executable })
if ($LASTEXITCODE -ne 0 -or $harness.Count -ne 1) { throw 'Acceptance harness build failed.' }
pwsh -NoProfile -File .\tests\scripts\WindowsRelease.Smoke.ps1 `
    -ReleaseDirectory artifacts/windows -DiscoveryHarness $harness[0].executable
if ($LASTEXITCODE -ne 0) { throw 'CPU release acceptance failed.' }
```

`-MediaCache <path>` selects a prepopulated cache, defaulting to `.scratch/media-tools-cache`. This check is always offline. The script installs into a unique scratch directory, then uninstalls and verifies owned-file removal, registration cleanup and unrelated-file preservation. It clears PATH only in child processes, not in the developer's shell.

For **both portable and installed** payloads, the harness uses Apollo's production discovery, probe, feature executors and output publication with a one-second 320x180/two-audio-track source:

| Acceptance | Required result |
| --- | --- |
| Probe | One H.264 video and two AAC tracks |
| Audio Mix | One AAC track, unchanged video codec/dimensions; gain and normalized variants |
| CPU Upscale | H.264 and H.265, 640x360, 8-bit, both audio tracks retained |
| Remux | MKV with the source stream-kind/codec inventory |
| Audio Extract | Original MKA, AAC and FLAC, exactly one audio stream |
| Frame Capture | PNG and JPEG at 500 ms, source dimensions, no audio |
| Real NVENC failure | Fresh child with `CUDA_VISIBLE_DEVICES=-1`; compiled encoders fail real probes, both Auto encodes resolve to CPU and complete; explicit NVENC refuses with the retained reason |

This failure fixture uses the unmodified pinned binary. It proves no-device initialization failure handling, not behavior on a particular obsolete driver. Native-window interaction, image quality and full-length source coverage remain separate manual checks. Do not describe this command-level evidence as a rendered UI review.

### NVIDIA baseline gate

The pin's [build evidence](../distribution/media-tools-acceptance.md) selects driver **566.14** on an **RTX 3060 Laptop GPU**, with `ffnvcodec n12.2.72.0-1-g9934f17` defining NVENC API **12.2**. This is not the oldest supported driver measurement and does not certify other GPUs. Encoder inventory and `ffmpeg -version` alone cannot establish this baseline.

On that baseline, rerun the same artifact check with mandatory GPU success:

```powershell
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
pwsh -NoProfile -File .\tests\scripts\WindowsRelease.Smoke.ps1 `
    -ReleaseDirectory artifacts/windows -DiscoveryHarness $harness[0].executable -RequireNvenc
if ($LASTEXITCODE -ne 0) { throw 'NVIDIA release acceptance failed.' }
```

This adds both successful real NVENC probes, explicit H.264/H.265 Upscale at **CQ 16**, and Auto choosing NVENC for each codec. Every output is probed for codec/dimensions, and process failure is fatal. Confirm there is no required-NVENC-API-newer-than-driver failure. If a deliberately incompatible-driver machine is available, also record its explicit NVENC diagnostic and Auto CPU result; do not alter a user's driver merely to obtain that evidence.

GPU success is a **manual publication gate**, not a GitHub-hosted CI requirement. Record date, source revision/local changes, Windows build, GPU/driver, manifest and executable hashes, installer/ZIP hashes, commands, results and unverified cases. Rebuilt artifacts need new artifact acceptance, even when the FFmpeg pin is unchanged. See the [v0.5.0 candidate acceptance record](RELEASE_ACCEPTANCE_0.5.0.md) for local evidence, not certification of a future tagged build.

## Updating the pin and redistribution review

Any change to archive URL, SHA-256, build identity or FFmpeg source version invalidates the old build acceptance, even if encoders are unchanged:

1. Select one exact archive with a versioned/immutable URL. Independently measure its SHA-256 and inspect its source/build and dependency identity. Never replace 7.0.2 merely because a newer binary has similar features.
2. Use a fresh cache and candidate manifest to stage both tools. Verify versions, `libx264`, `libx265`, `h264_nvenc`, `hevc_nvenc` and the complete required encoder inventory. Run the focused PowerShell checks, including corrupt-cache/offline behavior.
3. Run the installed and portable CPU smoke, both real NVENC probes and short H.264/H.265 CQ 16 Upscale on the selected baseline. Verify no newer API requirement, failure diagnostics and Auto fallback. Record evidence before accepting the new pin.
4. Review required licenses, distributor attribution, source references, static dependencies and corresponding-source delivery. Retain distribution material byte-for-byte; update `THIRD_PARTY_NOTICES.md` and the build/artifact acceptance records. The staged `licenses/ffmpeg/media-tools.json` must equal the accepted repository manifest.
5. Commit the accepted manifest, evidence, notices and any required staging changes together. Rebuild both artifacts, verify their hashes and rerun artifact acceptance before publication. Never rely on a cache key, version label or old GPU test to bless new bytes.

Both formats retain Apollo's root `LICENSE`, `THIRD_PARTY_NOTICES.md`, and the distributor's GPL v3 `LICENSE` and `README.txt`. The latter records FFmpeg source commit `e3a61e9103`, configuration and library versions. The [FFmpeg licensing guidance](https://ffmpeg.org/legal.html) is a starting point for the release review. The FFmpeg commit link does not supply all corresponding source for the static distribution. Before publishing, verify and document the source/build-material delivery for the exact bundled dependencies; preserve accessible source locations in the release notes. License-file presence and smoke tests do not establish completion of that review.

## Publish

Review or add the release notes while the GitHub Release is still a draft. Publish manually only after checksum/payload review, CPU acceptance, baseline NVIDIA acceptance, native installer/portable review and redistribution/source-delivery review have been recorded for the attached artifacts. Any unavailable required gate keeps the draft unpublished.

Publication is deliberately not automated.

## Reruns and failed releases

A failed tag workflow may be rerun from GitHub Actions. If a draft release already exists for the tag, a successful rerun refreshes its three release assets.

The workflow refuses to replace assets on an already published GitHub Release. If a published release is wrong, prepare a new version and tag instead of rewriting the published release.

If release verification fails before the draft is created, fix the release preparation on `main`, let CI pass for the corrected commit, and create the appropriate new tag only when that commit satisfies the release checks.

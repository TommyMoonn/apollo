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
LICENSE
THIRD_PARTY_NOTICES.md
```

Both distribution forms carry the tracked root `LICENSE`. FFmpeg and FFprobe remain external runtime requirements and are not bundled.

`SHA256SUMS.txt` contains lowercase SHA-256 entries for the installer and portable ZIP, in that order.

## Prepare the release

Run release preparation from a clean Windows clone with Git, PowerShell 7, and the Rust toolchain selected by `rust-toolchain.toml` available.

Set the intended version. The script updates `Cargo.toml` and the root Apollo entry in `Cargo.lock`, validates the result with locked Cargo metadata, and rolls both files back if it fails.

```powershell
$version = '0.1.0'
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

Commit the release preparation and get that exact commit onto `main`. Push it, then wait for the `CI` workflow on the `main` push to succeed. The tag workflow later requires a successful push-triggered `CI` run for the exact tagged commit.

Before creating the tag, verify the working tree is clean and local `HEAD` matches `origin/main`:

```powershell
git fetch origin main
git status --short

git rev-parse HEAD
git rev-parse origin/main
```

`git status --short` must produce no output, and the two commit IDs must be identical.

## Optional pre-tag Windows artifact check

The manually dispatched `Windows Installer Build` workflow runs the same Windows build and staging owner used by tagged releases. It uploads one Actions artifact named `Apollo-windows-x64` containing the three staged release files.

That Actions artifact is temporary build output with short retention. It is **not** a GitHub Release and does not publish anything to users.

Use it when you want to inspect the Windows installer and portable package before creating a tag.

## Tag the release

Create the version tag only after the release commit is on `main` and its push-triggered CI run has succeeded:

```powershell
git tag $tag
git push origin $tag
```

Pushing `v*` starts the `Draft GitHub Release` workflow. Before building, the workflow verifies that:

- the tag exactly matches the Cargo package version;
- `CHANGELOG.md` has the dated release section and matching version link;
- the tagged commit is contained in `main`;
- the exact tagged commit has a successful push-triggered `CI` run on `main`.

The workflow then builds the Windows GUI x64 release, stages the installer, portable ZIP, and checksum file, verifies the staged checksums, and creates or refreshes a **draft** GitHub Release titled `Apollo <tag>`.

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
- the install contains `Apollo.exe`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, and `Uninstall.exe`;
- an Apollo shortcut appears in the current user's Start Menu;
- Apollo appears in Windows Installed Apps with a working uninstaller;
- Apollo launches without an attached console window;
- uninstall removes Apollo's installed files and registration without removing unrelated user files.

For an upgrade check, install the new draft over an existing Apollo per-user installation and confirm the same Apollo registration is updated rather than creating a second application entry.

### Verify the portable ZIP

Expand `Apollo-windows-x64.zip` into an empty directory and confirm that it contains exactly:

```text
Apollo.exe
LICENSE
THIRD_PARTY_NOTICES.md
```

The portable form must run without creating an Apollo Installed Apps entry or Start Menu shortcut.

## Publish

Review or add the release notes while the GitHub Release is still a draft. When the notes, installer behavior, portable archive, and checksums are correct, publish the draft manually in GitHub.

Publication is deliberately not automated.

## Reruns and failed releases

A failed tag workflow may be rerun from GitHub Actions. If a draft release already exists for the tag, a successful rerun refreshes its three release assets.

The workflow refuses to replace assets on an already published GitHub Release. If a published release is wrong, prepare a new version and tag instead of rewriting the published release.

If release verification fails before the draft is created, fix the release preparation on `main`, let CI pass for the corrected commit, and create the appropriate new tag only when that commit satisfies the release checks.

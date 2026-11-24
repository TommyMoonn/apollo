# Apollo Repository Scripts

Apollo repository commands target **PowerShell 7**. `Cli.Common.ps1` is the single argument-parsing owner, while `Repository.Common.ps1` owns shared repository slug, path-safety, Git, and import-backup primitives.

## CLI conventions

Public commands use:

- `#requires -Version 7.0`
- `Set-StrictMode -Version Latest`
- `$ErrorActionPreference = "Stop"`
- lowercase `--kebab-case` canonical long options
- short aliases where defined
- compatibility aliases for the established PowerShell workflow
- `-h` and `--help`
- duplicate and unknown option rejection through `Cli.Common.ps1`

Do not add script-local argument parsers. The shared CLI, export and review commands are adapted from [Archeion at `4b711fc2`](https://github.com/TommyMoonn/archeion/tree/4b711fc2af4c70e321edd8b9072527c08c13fbf2/scripts) under GPL-3.0-only. Apollo retains its stricter option validation and Git inventory, writes workspace files directly to the final temporary ZIP, and uses a no-overwrite publish step.

For the end-to-end release operator workflow, see [`docs/RELEASES.md`](../docs/RELEASES.md).

## Commands

### Repository and workspace exports

```powershell
just zip repo
just zip workspace
just zip workspace --output 'C:/Exports/apollo workspace.zip'
```

The equivalent direct command is `pwsh -NoProfile -File scripts/zip-project.ps1 --mode repo` (or `--mode workspace`). `--mode` is required. Both commands accept `-p` / `--project` to select a different project root and `-o` / `--output` to select an external ZIP. Existing archives, non-ZIP destinations, and paths inside the project are refused.

- `repo` exports a committed Git revision, excluding dirty, untracked and ignored work. It refuses committed local planning or project records. Default: `apollo-repo(yyMMddHHmm).zip`.
- `workspace` exports current files using `.zipignore`, including eligible local records; no Git commit is required. Default: `apollo-workspace(yyMMddHHmm).zip`.

Both archives are flat and include a single `EXPORT_MANIFEST.json` (schema version 1) with mode, source commit, Git-visible dirty state, source baseline, actual local-record inclusion and UTC creation time. A workspace without a repository at its root has null commit and dirty state. The manifest is reserved; a source collision is refused. Output is published only after the completed archive is verified.

Workspace exclusions use Apollo's existing matcher, not Windows bsdtar. The shipped rules use bsdtar-compatible `^` root anchors and bare basename patterns for sensitive and temporary files. Root anchors match a file or directory at the workspace root, without excluding similarly named nested source paths. Apollo also supports its legacy pattern syntax: leading `/` or internal separators anchor patterns to the root, trailing `/` matches directories only, and unanchored bare names match at any depth. `*` and `?` stay within path segments, `**` spans segments (including zero directories in `**/`), and negation is unsupported. Windows matching is case-insensitive. Excluded directories are not traversed. Non-excluded links and reparse points are refused.

Local `.plans/` (including legacy `.planning/`), `.project/` and skills remain eligible for workspace snapshots. Review these records before sharing. `.zipignore` does not apply to Git archive mode; local records and secrets should never be committed.

### Review the working tree

```powershell
just review
just review --files
just review --project 'C:/Projects/Apollo Copy' --files
```

The direct equivalent is `pwsh -NoProfile -File scripts/review-changes.ps1 --files`. The default output includes branch, counts by status and area, staged/unstaged tracked change counts and review flags. A rename counts as one change; a path with both staged and unstaged edits counts in both. Untracked paths are counted separately. `--files` lists paths and their index/worktree states. Index changes reversed in the worktree are reported as index/worktree-only changes, not silently omitted. Warnings cover deletions, test coverage review (including inline Rust tests), generated/sensitive paths, Cargo dependencies, CI, files at least 1 MiB and staged/unstaged whitespace issues. Warnings are advisory; inspection failures return errors.

### Package changed files

```powershell
just package phase-f01.2
just package phase-f01.2 --tracked-only --output 'C:/Exports/apollo patch.zip'
```

The package contains the working-tree delta against committed HEAD, including staged and unstaged changes and non-ignored untracked files. Use `--tracked-only` to exclude untracked files. Local records, owner-managed skills, generated output and sensitive paths are excluded. Review the whole delta before sharing; the exporter does not identify phase ownership.

Defaults: `apollo-<slug>-changed-files.zip` beside the project, with `changes` as the default slug. `--project` selects the source root, `--output` overrides the destination and `--force` permits replacement only after the new archive is complete. A clean delta produces no archive. Root `.apollo-change-package.json` records the source commit, tracked-only mode, counts and UTC creation time. Root `.chatgpt-delete-manifest.txt` lists deleted paths and rename sources when needed.

### Apply changed files

```powershell
just apply --zip 'C:/Downloads/apollo-task-changed-files.zip' --dry-run
just apply --zip 'C:/Downloads/apollo-task-changed-files.zip'
```

Without `--zip`, discovery selects the newest `apollo-*-changed-files.zip` in Downloads, including browser copies such as ` (1)`. `--downloads` and `--pattern` narrow the search. Snapshots are for extraction, not application. Explicit legacy changed-file ZIPs are supported, including wrapped layouts (`--strip-root` when needed) and deletion-only archives. The importer validates provenance and counts when available, warns if the source commit is missing, and never imports metadata. It validates all targets before mutation, refuses protected or linked paths, skips identical files, and refuses tracked dirty state unless `--allow-dirty` is set. `dir:` deletion entries require `--allow-directory-deletion`.

The default backup includes replaced/deleted content and a journal of import-created paths. `--no-backup` explicitly disables recovery; dry runs never write targets or backups.

### Restore an import backup

```powershell
just restore --dry-run
just restore
just restore --backup 'C:/path/to/backup'
```

New backups live under `%LOCALAPPDATA%/Apollo/ChatGPTImports/<project-slug>/<timestamp>-<id>` (with a local application data/temp fallback). Omit `--backup` to discover the newest matching backup. Restore preflights conflicts, reinstates originals, removes unchanged import-created files and removes only empty recorded directories. Unrelated content is preserved. The journal is written before each operation and retained after restore for recovery or retry. Empty directories without a confirmed creation receipt, or whose creation timestamp has changed on Windows, are preserved. A completed restore is not replayed. Resolve reported later-edit conflicts before retrying an incomplete restore. Legacy backups under `ChatGPTImports` remain supported with a warning: they cannot detect later edits or remove files created by the import.

Direct commands: `scripts/package-changes.ps1`, `scripts/apply-changes.ps1` and `scripts/restore-changes.ps1` accept the same options and legacy flag aliases; run each with `--help` for details. Adapted from Archeion commit `4b711fc2` (GPL-3.0), with Apollo's repository inventory, stricter path/ZIP preflight, safe archive replacement and extended rollback journal.

### Clean generated output

```powershell
.\scripts\clean-generated.ps1 --rust
.\scripts\clean-generated.ps1 --rust --dry-run
```

Rust cleanup targets `target/`. Tracked files are protected unless `--force` is explicitly supplied. The retired bundle cleanup interface remains temporarily compatible pending F01.3; offline preparation and maintenance are owner-managed.

### Validate release metadata

```powershell
.\scripts\check-release.ps1
.\scripts\check-release.ps1 --tag v0.5.0
.\scripts\check-release.ps1 --tag v0.5.0 --release-changelog
```

Checks the `Cargo.toml` package version against the root `apollo` entry in `Cargo.lock`. An explicit release tag must equal `v<version>`. In a GitHub tag workflow, `GITHUB_REF_NAME` is used when no tag is supplied. `--release-changelog` additionally requires a dated release heading and matching version link in `CHANGELOG.md`. Validation is read-only.

### Set the release version

```powershell
.\scripts\set-version.ps1 0.5.0
```

Updates the `[package].version` in `Cargo.toml`, lets Cargo refresh the root package entry in `Cargo.lock`, verifies the result with locked Cargo metadata, and runs the shared release validator. If any step fails, both version files are restored.

### Stage pinned media tools

```powershell
.\scripts\stage-media-tools.ps1 --output .scratch/media-payload
.\scripts\stage-media-tools.ps1 --offline --output .scratch/media-payload-offline
```

Uses the exact archive URL and SHA-256 in `distribution/media-tools.json`. The default cache is `.scratch/media-tools-cache`; `--cache` selects another cache. Staging verifies the archive before extraction, validates both executable versions and required encoders, and publishes only the pair, required redistribution material and pinned metadata. It requires a new output directory and no NVIDIA GPU. Offline mode requires a valid cached archive and never downloads. Explicit relative paths resolve from the current directory.

### Stage Windows release artifacts

```powershell
.\scripts\stage-windows-release.ps1
```

Validates the current release metadata, verifies the Windows GUI x64 `apollo.exe`, builds the repository-owned NSIS installer, and atomically recreates `artifacts/windows/` with:

```text
Apollo-Setup-x64.exe
Apollo-windows-x64.zip
SHA256SUMS.txt
```

The installer is the primary Windows distribution artifact. Both packages include `Apollo.exe`, `ffmpeg.exe`, `ffprobe.exe`, the root `LICENSE`, `THIRD_PARTY_NOTICES.md`, and `licenses/ffmpeg/{LICENSE,README.txt,media-tools.json}`. Release staging supplies one verified media-tool payload to both packaging consumers through `stage-media-tools.ps1`. `SHA256SUMS.txt` contains lowercase SHA-256 entries for the installer and ZIP, in that order.

By default the executable is read from `target/x86_64-pc-windows-msvc/release/apollo.exe`; use `--executable` and `--output` for explicit paths. Installer creation uses the same `build-windows-installer.ps1` owner described below; pass `--makensis` when the NSIS compiler is not discoverable automatically. Relative paths resolve from the selected project root. A failed installer, ZIP, or checksum build leaves any previously staged release directory unchanged. The generated `artifacts/` tree is ignored by Git and full-project snapshots.

Use `--media-cache <path>` to select the archive cache and `--offline` to require a valid cached archive. These options also apply to standalone installer builds. Media-tool staging failure leaves the previous release output intact.

### Build the Windows installer

```powershell
.\scripts\build-windows-installer.ps1
```

Builds Apollo's per-user NSIS installer from the validated Windows GUI x64 release executable. The default output is:

```text
artifacts/windows-installer/Apollo-Setup-x64.exe
```

Install NSIS 3.11 so `makensis.exe` is available on `PATH`, or pass its path explicitly with `--makensis`. The wrapper derives the installer version from `Cargo.toml`, validates the release executable through the same PE32+/x64/GUI-subsystem owner used by release staging, and includes the verified media-tool pair and license material. Use `--executable` and `--output` to override the build input or destination; the destination filename must remain `Apollo-Setup-x64.exe`.

The installer is per-user, installs under `%LOCALAPPDATA%\Programs\Apollo`, and creates an Apollo Start Menu shortcut and Installed Apps entry, but no desktop shortcut. Uninstall removes its owned media tools and license files without removing unrelated user files. The installer, uninstaller, installed executable, Start Menu shortcut, and Installed Apps entry use the repository-owned Apollo icon.

## Tests

Run all PowerShell tooling tests from the repository root:

```powershell
pwsh -NoProfile -File .\tests\scripts\run.ps1
```

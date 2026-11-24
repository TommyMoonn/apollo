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

For installed and portable artifact checks, see [Testing](../docs/TESTING.md#windows-artifact-acceptance).

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
just clean
just clean --dry-run
just clean --project 'C:/Projects/Apollo Copy' --dry-run
```

The direct equivalent is `pwsh -NoProfile -File scripts/clean-generated.ps1`. Cleanup selects only the root `target/` directory; `--rust` remains an explicit alias for that selection. `--dry-run` reports the path and estimated size without deleting anything. `--force` permits tracked generated files to be removed and prints a warning. It does not bypass root, reparse-point or protected-content checks. The project must be a Git worktree root or an extracted Cargo project containing `Cargo.toml`.

Local plans, `.agent-work/`, legacy `.dev/`, offline archives and staged release artifacts are not cleanup targets. The retired bundle and all-target flags are no longer supported. Offline preparation and maintenance are owner-managed.

Adapted from Archeion commit `4b711fc2` (GPL-3.0-only): Rust output is the only target, with Apollo's shared repository checks, link-safe size traversal and shared CLI reporting.

### Generate Rust coverage

```powershell
pwsh -NoProfile -File scripts/coverage-rust.ps1 --install
just coverage
```

`--install` explicitly installs the pinned coverage prerequisites. The normal command runs `scripts/coverage-rust.ps1` without installation, validates the selected toolchain and `cargo-llvm-cov` version, and instruments the ordinary locked Windows x64 Rust tests once. The HTML and terminal summary use the same collected data. Ignored external-media tests stay opt-in.

Reports are written to `.scratch/coverage/<run-id>/` and the HTML index path is printed. Instrumented build and profile data remain in `target/llvm-cov-target/`. See [Rust coverage diagnostics](../docs/DEVELOPMENT.md#rust-coverage-diagnostics) for prerequisites, report interpretation and CI behavior. Coverage is diagnostic only and has no required percentage or release gate.

### Validate release metadata

```powershell
just check-release
just check-release --tag v0.7.0 --require-changelog
```

The direct equivalent is `pwsh -NoProfile -File scripts/check-release.ps1`. It accepts `--project <path>` for an alternate project root. Validation is read-only: the Cargo manifest and root lock entry must agree on a stable `X.Y.Z` version with components from 0 to 65535. A supplied tag must exactly equal `v<version>` and agree with inherited GitHub tag context, if present. Normal checks do not require a release note or changelog entry.

`--require-changelog` requires a supplied or inherited tag, one dated release heading and matching comparison URL in `CHANGELOG.md`, and the corresponding canonical `release-notes/vX.Y.Z.md`. The legacy `--release-changelog` flag remains accepted for compatibility with owner-managed release tooling; it checks the changelog alone. Use the full candidate check for release preparation.

### Detect a release candidate

```powershell
just detect-release-candidate SHA TommyMoonn/apollo
```

The direct equivalent is `pwsh -NoProfile -File scripts/detect-release-candidate.ps1 SHA TommyMoonn/apollo`; replace `SHA` with the full lowercase checkout SHA. Use `--project <path>` for an alternate clean checkout with complete history. The script also accepts `--commit` and `--repo` instead of positional arguments. It checks stable first-parent version transitions, candidate metadata and exact remote tag/release identity; an increase requires authenticated `gh` access with release visibility. GitHub may hide drafts from restricted credentials, so publication must recheck identity with its authorized credential. JSON and optional `GITHUB_OUTPUT` fields report `candidate`, `version`, `sha` and `reason`. Failed checks exit nonzero without emitting a candidate. It neither creates tags/releases nor checks CI authorization. See [Releasing](../docs/RELEASING.md#detect-release-candidates-read-only).

### Publish a verified candidate (workflow-only)

`publish-release.ps1` is reserved for the authorized publication job. It accepts `--commit`, `--version`, `--repo`, `--artifacts-dir` and optional `--project`. It requires GitHub Actions repository context, `GH_TOKEN`, a clean checkout with full history and the original independently verified Windows artifacts. It verifies local candidate metadata and artifact integrity again, resolves the exact remote tag and draft, reconciles assets by digest, and publishes only a complete matching release. A published replay performs no writes. There is no Just recipe or local publishing procedure; use [Publication recovery](../docs/RELEASING.md#publication-recovery) for conflict and retry policy.

### Set the release version

```powershell
just set-version 0.8.0
```

The direct equivalent is `pwsh -NoProfile -File scripts/set-version.ps1 0.8.0`; use `--project <path>` for another project. Cargo owns the root lockfile update. The command checks that only the Apollo root version changed (allowing Cargo's line-ending normalization) and restores both files if any step fails. The example version is illustrative; the command does not write notes, tags or releases. See [Releasing](../docs/RELEASING.md) for the operator sequence.

The focused release-policy fixture check can be run directly with `pwsh -NoProfile -File tests/scripts/ReleasePolicy.Tests.ps1`. It remains optional and separate from required CI, `just verify` and `just test-scripts`.

Adapted from Archeion commit `4b711fc2` (GPL-3.0-only), retaining Cargo as Apollo's only maintained version source and validating Windows resources and Apollo's two-download note format without Node or a website.

### Stage pinned media tools

```powershell
just stage-media-tools --output .scratch/media-payload
just stage-media-tools --offline --output .scratch/media-payload-offline
```

Uses the exact archive URL and SHA-256 in `distribution/media-tools.json`. The default cache is `.scratch/media-tools-cache`; `--cache` selects another cache. Staging verifies the archive before extraction, validates both executable versions and required encoders, and publishes only the pair, required redistribution material and pinned metadata. It requires a new output directory and no NVIDIA GPU. Offline mode requires a valid cached archive and never downloads. Explicit relative paths resolve from the current directory.

### Stage and verify Windows release artifacts

```powershell
just stage-release
```

Validates the current release metadata, verifies the Windows GUI x64 `apollo.exe`, builds the repository-owned NSIS installer, and atomically recreates `artifacts/windows/` with:

```text
Apollo-Setup-x64.exe
Apollo-windows-x64.zip
SHA256SUMS.txt
release-provenance.json
```

`release-provenance.json` is internal to artifact transfer; the other three files are the public release assets. Staging validates the complete set before replacement and refuses unrelated content, linked destinations and tracked output. Both `--expected-sha` and `--expected-version` are required to record candidate identity; without them the set is local-only. Use `just verify-release --local --expected-version X.Y.Z` for a local build, or `just verify-release --expected-sha <sha> --expected-version X.Y.Z --artifacts-dir <path>` for a clean candidate. The verifier checks exact artifact names, checksums, provenance, portable entries and Apollo's resource identity without rebuilding or downloading tools. It does not execute the installer. See [Releasing](../docs/RELEASING.md).

The installer is the primary Windows distribution artifact. Both packages include `Apollo.exe`, `ffmpeg.exe`, `ffprobe.exe`, the root `LICENSE`, `THIRD_PARTY_NOTICES.md`, and `licenses/ffmpeg/{LICENSE,README.txt,media-tools.json}`. Release staging supplies one verified media-tool payload to both packaging consumers through `stage-media-tools.ps1`. `SHA256SUMS.txt` contains lowercase SHA-256 entries for the installer and ZIP, in that order.

By default the executable is read from `target/x86_64-pc-windows-msvc/release/apollo.exe`; use `--executable` and `--output` for explicit paths. Installer creation uses the same `build-windows-installer.ps1` owner described below; pass `--makensis` when the NSIS compiler is not discoverable automatically. Relative paths resolve from the selected project root. A failed installer, ZIP, or checksum build leaves any previously staged release directory unchanged. The generated `artifacts/` tree is ignored by Git and full-project snapshots.

Use `--media-cache <path>` to select the archive cache and `--offline` to require a valid cached archive. These options also apply to standalone installer builds. Media-tool staging failure leaves the previous release output intact.

### Build the Windows installer

```powershell
just build-installer
```

Builds Apollo's per-user NSIS installer from the validated Windows GUI x64 release executable. The default output is:

```text
artifacts/windows-installer/Apollo-Setup-x64.exe
```

Install NSIS 3.11 so `makensis.exe` is available on `PATH`, or pass its path explicitly with `--makensis`. The wrapper derives the installer version from `Cargo.toml`, validates the release executable through the same PE32+/x64/GUI-subsystem owner used by release staging, and includes the verified media-tool pair and license material. Use `--executable` and `--output` to override the build input or destination; the destination filename must remain `Apollo-Setup-x64.exe`.

The installer is per-user, installs under `%LOCALAPPDATA%\Programs\Apollo`, and creates an Apollo Start Menu shortcut and Installed Apps entry, but no desktop shortcut. Uninstall removes its owned media tools and license files without removing unrelated user files. The installer, uninstaller, installed executable, Start Menu shortcut, and Installed Apps entry use the repository-owned Apollo icon.

### Run native acceptance

```powershell
just smoke-installer --local --expected-version 0.7.0
just smoke-installer --expected-sha <sha> --expected-version X.Y.Z --artifacts-dir <path>
just test-media --media-dir .scratch/dev-media
```

`smoke-windows-installer.ps1` delegates to `tests/scripts/WindowsRelease.Smoke.ps1`. It verifies staged artifacts before exercising clean install, portable processing, uninstall and a checksum-verified published predecessor. `--upgrade-only` requires an older version instead of a same-version reinstall. `--discovery-harness <path>` reuses a matching prebuilt harness, and `--require-nvenc` adds real GPU success checks. Use a clean Windows x64 user or VM and GitHub CLI access to published Apollo releases. The original application binary and pinned media cache are not required.

`just test-media` uses the same harness against an explicitly selected pair (default `.scratch/dev-media`), without installation. It accepts `--media-dir`, `--discovery-harness` and `--require-nvenc`. Scratch output is isolated under `.scratch/release-smoke-*`; see [Testing](../docs/TESTING.md#windows-artifact-acceptance) for cleanup and failure behavior. Neither recipe is part of `just verify`.

## Optional tooling smoke check

Run `just test-scripts` (or `pwsh -NoProfile -File tests/scripts/run.ps1`) to check PowerShell syntax, workspace export and review, changed-file transfer and rollback, unsafe ZIP rejection, and cleanup protection in disposable repositories. It reports a failure when a check fails but is not part of `just verify`, CI or release packaging.

Actual release metadata, artifact integrity and installed/portable payload checks remain part of their respective release workflows.

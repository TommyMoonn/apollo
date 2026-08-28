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

Do not add script-local argument parsers.

## Commands

### Full-project snapshot

```powershell
.\scripts\zip-project.ps1
```

Creates `<project-slug>(yyMMddHHmm).zip` beside the repository. The command is intentionally zero-config and respects `.zipignore`.

### Review the working tree

```powershell
.\scripts\review-changes.ps1
.\scripts\review-changes.ps1 --files
```

Summarizes tracked and non-ignored untracked changes against `HEAD`, including staged/unstaged state, renames, deletions, and Rust-repository categories.

### Package changed files

```powershell
.\scripts\package-changes.ps1 --name 0.1.0.5-10
```

Packages tracked changes plus non-ignored untracked files by default. Use `--tracked-only` to exclude untracked files. Tracked deletions and rename sources are represented in `.chatgpt-delete-manifest.txt` only when required.

Compatibility entry point:

```powershell
.\scripts\package-changed-files.ps1
```

### Apply a project ZIP

```powershell
.\scripts\apply-changes.ps1 --zip C:\path\to\apollo-task-changed-files.zip
```

Without `--zip`, the command searches Downloads for the newest `<project-slug>*.zip`. This accepts both full project snapshots such as `apollo(2608281142).zip` and changed-files packages such as `apollo-phase-0.1.0.56-changed-files.zip`, including browser duplicate names like ` (1)` and ` (2)`. Use `--pattern` to narrow discovery when needed. The importer rejects unsafe archive paths and `.git`, skips identical files, refuses tracked dirty state by default, supports dry runs, applies the deletion manifest, and backs up overwritten/deleted paths unless `--no-backup` is used.

Compatibility entry point:

```powershell
.\scripts\apply-chatgpt-zip.ps1
```

### Restore an import backup

```powershell
.\scripts\restore-changes.ps1
.\scripts\restore-changes.ps1 --backup C:\path\to\backup
```

Uses the newest project backup when `--backup` is omitted. Import backups contain overwritten and deleted paths only, so files newly created by an import are not automatically removed during restoration.

Compatibility entry point:

```powershell
.\scripts\restore-chatgpt-import.ps1
```

### Clean generated output

```powershell
.\scripts\clean-generated.ps1 --rust
.\scripts\clean-generated.ps1 --chatgpt-bundles
.\scripts\clean-generated.ps1 --all --dry-run
```

The only initial cleanup targets are `target/` and `.dev/chatgpt/bundles/`. Tracked files are protected unless `--force` is explicitly supplied.

### Validate release metadata

```powershell
.\scripts\check-release.ps1
.\scripts\check-release.ps1 --tag v0.1.0
.\scripts\check-release.ps1 --tag v0.1.0 --release-changelog
```

Checks the `Cargo.toml` package version against the root `apollo` entry in `Cargo.lock`. An explicit release tag must equal `v<version>`. In a GitHub tag workflow, `GITHUB_REF_NAME` is used when no tag is supplied. `--release-changelog` additionally requires a dated release heading and matching version link in `CHANGELOG.md`. Validation is read-only.

### Set the release version

```powershell
.\scripts\set-version.ps1 0.2.0
```

Updates the `[package].version` in `Cargo.toml`, lets Cargo refresh the root package entry in `Cargo.lock`, verifies the result with locked Cargo metadata, and runs the shared release validator. If any step fails, both version files are restored.

### Stage Windows release artifacts

```powershell
.\scripts\stage-windows-release.ps1
```

Validates the current release metadata, verifies the Windows x64 `apollo.exe`, and recreates `artifacts/windows/` with:

```text
Apollo-windows-x64.zip
SHA256SUMS.txt
```

The ZIP contains only `Apollo.exe`, `LICENSE`, and `THIRD_PARTY_NOTICES.md`. FFmpeg and FFprobe remain external runtime requirements and are not bundled. By default the executable is read from `target/x86_64-pc-windows-msvc/release/apollo.exe`; use `--executable` and `--output` for explicit paths. Relative paths resolve from the selected project root. The generated `artifacts/` tree is ignored by Git and full-project snapshots.

## Tests

Run all PowerShell tooling tests from the repository root:

```powershell
pwsh -NoProfile -File .\tests\scripts\run.ps1
```

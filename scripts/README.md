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

### Apply a changed-files package

```powershell
.\scripts\apply-changes.ps1 --zip C:\path\to\apollo-task-changed-files.zip
```

Without `--zip`, the command searches Downloads for the newest `<project-slug>-*-changed-files.zip`, including Windows duplicate-download names such as `...-changed-files (1).zip`, `(2).zip`, and repeated/no-space suffixes such as `...-changed-files(1)(1).zip`. It rejects unsafe archive paths and `.git`, skips identical files, refuses tracked dirty state by default, supports dry runs, applies the deletion manifest, and backs up overwritten/deleted paths unless `--no-backup` is used.

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

## Tests

Run all PowerShell tooling tests from the repository root:

```powershell
pwsh -NoProfile -File .\tests\scripts\run.ps1
```

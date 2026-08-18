# Apollo Development

## Prerequisites

Apollo v0.1.0 targets Windows as its primary development and validation platform.

Required development tools:

- Git
- Rust toolchain pinned by `rust-toolchain.toml`
- PowerShell 7 for repository scripts as they are introduced
- FFmpeg and FFprobe for media functionality once the media phases begin

Rustup is the recommended way to install the pinned Rust toolchain. From the repository root, the toolchain file should select the required version automatically.

FFmpeg and FFprobe are external runtime tools. Bootstrap and early foundation phases do not require them to run, but later media phases will discover and validate them through one shared application owner rather than ad hoc feature-specific checks.

## Build

Build the native application:

```powershell
cargo build
```

For a faster compile-only validation when a runnable binary is not required:

```powershell
cargo check
```

## Test

Run the Rust test suite with:

```powershell
cargo test --all-targets
```

For early bootstrap phases where no target-specific tests exist yet, `cargo test` is also sufficient for the phase-specific gate when the plan explicitly says so.

Run the PowerShell tooling tests with the repository-owned runner:

```powershell
pwsh -NoProfile -File .\tests\scripts\run.ps1
```

## Formatting

Check Rust formatting:

```powershell
cargo fmt --check
```

Apply Rust formatting locally when needed:

```powershell
cargo fmt
```

Do not use formatting changes as an excuse for unrelated cleanup outside the active phase.

## Clippy

The standard lint gate is:

```powershell
cargo clippy --all-targets --all-features -- -D warnings
```

Warnings are treated as errors for the project gate.

## Standard Rust Gate

Before completing an application-code phase, run:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Report checks truthfully. If a required tool or dependency is unavailable, record the check as not run rather than inferring success.

## FFmpeg Expectations

Apollo uses FFmpeg and FFprobe as external media engines instead of reimplementing codecs or linking libav during v0.1.0.

Repository rules for media execution:

- never build shell command strings
- pass process arguments separately
- use probed stream indexes for stream mappings
- do not silently re-encode unrelated streams
- process exit is authoritative for success
- cancellation must terminate only the owned child process
- failed or cancelled jobs must not publish misleading partial outputs

FFmpeg discovery, probing, command construction, process lifecycle, progress parsing, and output publication each receive an explicit shared owner in later phases.

## Repository Scripts

Apollo uses PowerShell 7 repository tooling with two shared owners:

- `scripts/Cli.Common.ps1` parses public CLI arguments.
- `scripts/Repository.Common.ps1` owns shared repository slug derivation, safe project-relative path resolution, Git process invocation, and ChatGPT import backup location.

Canonical commands:

```text
apply-changes.ps1
package-changes.ps1
review-changes.ps1
restore-changes.ps1
clean-generated.ps1
zip-project.ps1
```

Compatibility entry points remain logic-free wrappers:

```text
apply-chatgpt-zip.ps1
package-changed-files.ps1
restore-chatgpt-import.ps1
```

Public command conventions:

- PowerShell 7
- `Set-StrictMode -Version Latest`
- `$ErrorActionPreference = "Stop"`
- lowercase `--kebab-case` long options
- `-h` and `--help` on every public command
- consistent shared option names
- no duplicated script-local argument parsing

### Review and changed-files workflow

Review first:

```powershell
.\scripts\review-changes.ps1 --files
```

Then package the current tracked changes and intended non-ignored untracked files:

```powershell
.\scripts\package-changes.ps1 --name <task-or-phase>
```

Use `--tracked-only` when untracked files must be excluded. Packages preserve project-relative paths. Tracked deletions and rename sources are represented by `.chatgpt-delete-manifest.txt` only when required.

Default package naming is:

```text
apollo-<task-or-phase>-changed-files.zip
```

Apply a package explicitly:

```powershell
.\scripts\apply-changes.ps1 --zip <path>
```

Or omit `--zip` to select the newest `<project-slug>-*-changed-files.zip` from Downloads. The importer rejects absolute/traversal paths and `.git`, refuses tracked dirty state by default, skips identical files, supports dry run, requires explicit opt-in for directory deletion, and backs up overwritten/deleted paths by default under:

```text
%LOCALAPPDATA%\ChatGPTImports\<project-slug>\<timestamp>\
```

Restore the newest backup with:

```powershell
.\scripts\restore-changes.ps1
```

A backup records overwritten and deleted paths. It does not record files that were newly created by the import, so restoration does not automatically remove those new files.

### Full-project ZIP workflow

Create a full-project snapshot with:

```powershell
.\scripts\zip-project.ps1
```

The script derives the repository slug and creates a timestamped archive beside the project directory:

```text
apollo(yyMMddHHmm).zip
```

`.zipignore` controls exclusions. `.planning/` and `.project/` remain eligible for full-project snapshots even though Git ignores them.

### Generated-output cleanup

Preview or remove only recognized generated roots:

```powershell
.\scripts\clean-generated.ps1 --rust --dry-run
.\scripts\clean-generated.ps1 --chatgpt-bundles --dry-run
.\scripts\clean-generated.ps1 --all
```

Initial targets are only:

```text
target/
.dev/chatgpt/bundles/
```

The cleanup command refuses paths outside the repository, never touches `.git`, and protects tracked files unless `--force` is explicitly used.

## Local Workspace Roots

The repository reserves these local roots:

```text
.planning/   local implementation plans
.project/    local project records and reviews
.scratch/    disposable working data
```

They are intentionally ignored by Git and must never become build prerequisites. A clean clone must build and test without them.

Tracked development tooling belongs under `.dev/`. Generated ChatGPT dependency bundles belong under `.dev/chatgpt/bundles/` and are ignored.

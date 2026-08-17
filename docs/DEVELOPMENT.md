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

Apollo uses PowerShell 7 repository tooling modeled after the established project workflow.

Canonical command names planned for the tooling phases are:

```text
apply-changes.ps1
package-changes.ps1
review-changes.ps1
restore-changes.ps1
clean-generated.ps1
zip-project.ps1
```

Compatibility entry points planned for familiar legacy names are:

```text
apply-chatgpt-zip.ps1
package-changed-files.ps1
restore-chatgpt-import.ps1
```

The compatibility entry points must remain logic-free wrappers around the canonical commands.

Public command conventions:

- PowerShell 7
- `Set-StrictMode -Version Latest`
- `$ErrorActionPreference = "Stop"`
- lowercase `--kebab-case` long options
- `-h` and `--help` on every public command
- shared option meanings remain consistent across commands

Do not duplicate argument parsing between individual scripts. `scripts/Cli.Common.ps1` is the single argument-parsing owner.

## Changed-Files Workflow

The changed-files workflow is introduced in later tooling phases. Its intended flow is:

1. Review the working tree.
2. Package only the task- or phase-owned changed files.
3. Preserve project-relative paths.
4. Represent tracked deletions and relevant renames through `.chatgpt-delete-manifest.txt` when required.
5. Apply the package through the safe import script.
6. Restore from the import backup if necessary.

Planned canonical package naming:

```text
apollo-<task-or-phase>-changed-files.zip
```

Do not manually invent deletion manifests when no tracked deletion exists.

## Full-Project ZIP Workflow

The full-project snapshot command is introduced in a later tooling phase as:

```powershell
.\scripts\zip-project.ps1
```

The script will derive the repository slug and produce a timestamped archive beside the project directory:

```text
apollo(yyMMddHHmm).zip
```

`.zipignore` controls snapshot exclusions. In particular, generated build output, Git internals, ChatGPT bundle output, scratch data, logs, temporary files, and existing ZIPs are excluded.

`.planning/` and `.project/` are intentionally Git-ignored but are not excluded from full-project snapshots by default. They may carry useful local plans and project records into a ChatGPT project snapshot.

## Local Workspace Roots

The repository reserves these local roots:

```text
.planning/   local implementation plans
.project/    local project records and reviews
.scratch/    disposable working data
```

They are intentionally ignored by Git and must never become build prerequisites. A clean clone must build and test without them.

Tracked development tooling belongs under `.dev/`. Generated ChatGPT dependency bundles belong under `.dev/chatgpt/bundles/` and are ignored.

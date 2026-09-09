# Apollo Development

## Prerequisites

Apollo targets Windows as its primary development and validation platform.

Required development tools:

- Git
- Rust toolchain pinned by `rust-toolchain.toml`
- PowerShell 7 for repository scripts
- FFmpeg and FFprobe for media inspection, processing, and runtime smoke tests

Rustup is the recommended way to install the pinned Rust toolchain. From the repository root, the toolchain file should select the required version automatically.

FFmpeg and FFprobe are external runtime tools. Apollo discovers and validates them through one shared application owner rather than feature-specific checks.

## Releases

The operator workflow for versioning, tagging, Windows artifacts, draft GitHub Releases, verification, and publication is documented in [RELEASES.md](RELEASES.md).

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

## Typography

Apollo does not depend on machine-installed fonts for application typography. The repository font assets live under:

```text
assets/fonts/
```

`src/ui/fonts.rs` is the single owner for loading those assets, stable font-family names, the typography scale, and egui text-style mapping. IBM Plex Sans is the primary UI family; Fira Code is reserved for the Apollo wordmark and compact technical metadata. Individual screens should request the shared text styles rather than configuring fonts locally.

Missing or invalid required font assets are startup configuration errors. Apollo must not silently fall back to whatever fonts happen to be installed on the development machine.

## FFmpeg Expectations

Apollo uses FFmpeg and FFprobe as external media engines instead of reimplementing codecs or linking libav.

Repository rules for media execution:

- never build shell command strings
- pass process arguments separately
- use probed stream indexes for stream mappings
- do not silently re-encode unrelated streams
- process exit is authoritative for success
- cancellation must terminate only the owned child process
- failed or cancelled jobs must not publish misleading partial outputs

`src/media/tools.rs` owns FFmpeg/FFprobe discovery and bounded version validation. `src/media/probe.rs` owns FFprobe invocation, JSON deserialization, and normalization into the shared types in `src/media/model.rs`. `src/media/container.rs` owns shared MP4 and Matroska stream-copy compatibility. `src/media/process.rs` owns long-running child-process spawning, cancellation, lifecycle settlement, bounded diagnostic stderr capture, and nonblocking output/progress channels. `src/media/output.rs` owns validation, operation-owned temporary paths, success-only publication, and failed/cancelled cleanup. `src/media/progress.rs` owns parsing machine-readable FFmpeg progress records into shared progress state; process exit remains the success/failure authority.

Each workflow keeps its request model, validation, structured FFmpeg command construction, and execution policy under its feature module:

```text
src/audio_mix/
src/upscale/
src/remux/
src/audio_extract/
src/frame_capture/
```

The corresponding ready and lifecycle surfaces live under `src/ui/`. Remux remains stream copy only, Audio Extract maps one selected audio stream, and Frame Capture maps one video stream for one still image. Do not broaden these focused workflows into conversion, stream-editing, or timeline tools without an explicit release plan.

## Upscale Sizing, Encoding, and Format Policy

`src/upscale/sizing.rs` owns exact aspect-preserving even output dimensions. Preset boundaries remain orientation-aware. Custom boundaries are explicit positive even width/height values and are never swapped. Sizing rejects boxes that cannot enlarge the source without boundary overshoot or source-edge downscale from quantization. Execution uses the calculated dimensions, not raw UI input.

Scaling remains `scale=<width>:<height>:flags=lanczos`. Encoder selection does not enable hardware decoding or CUDA/NPP scaling.

| Backend | H.264 | H.265 | Quality policy |
| --- | --- | --- | --- |
| CPU | `libx264`, CRF 18 default | `libx265`, CRF 20 default | CRF 0..51, `slow` preset |
| NVIDIA NVENC | `h264_nvenc`, CQ 16 default | `hevc_nvenc`, CQ 16 default | CQ 1..51, `p5`, VBR, explicit CQ |

Lower quality numbers favor quality over file size. CRF and CQ are not interchangeable scales. State retains values independently for each codec/backend, alongside the last custom boundary and manual output name.

The shared media-tool owner checks compiled NVENC availability and runs bounded, hidden, minimal real encode probes off the egui thread. H.264 and H.265 results are independent and cached for the resolved FFmpeg identity. Base tool discovery can finish before these probes: normal workspace use and CPU Upscale remain available while Auto and explicit NVENC show checking state. Auto selects NVENC only after success; explicit unavailable NVENC retains diagnostics and never silently falls back. Do not probe on repaint or command construction.

### Pixel Format and Color Limits

The probe retains pixel format, derivable bit depth, range, matrix, transfer, and primaries. Shared Upscale validation resolves the output format for both ready feedback and execution:

- 8-bit `yuv420p` is supported on CPU and NVENC. `nv12` uses `yuv420p` on CPU or `nv12` on NVENC; `yuvj420p` is CPU-only.
- Supported 10-bit 4:2:0 sources require H.265, using `yuv420p10le` on CPU or `p010le` on NVENC. PQ/HLG signaling requires known compatible 10-bit input.
- Unsupported or contradictory known format information is rejected. When both pixel identity and depth are absent, strict FFmpeg format selection preserves the filter output rather than guessing an 8-bit format.
- Commands use strict `-pix_fmt` selection and map only available color values. External encoder/hardware limitations can still fail execution instead of silently choosing a different format.
- This is not a guarantee of full HDR preservation: mastering-display/content-light side data is not explicitly preserved or verified, and format checks do not establish visual fidelity.

### Runtime Regression Checks

The normal Rust gate covers sizing, encoder resolution, quality bounds, command construction, and format validation without requiring GPU hardware. Opt-in tests exercise real CPU/NVENC encoding and FFprobe verification:

```powershell
cargo test --test upscale real_cpu_upscale -- --ignored --nocapture
cargo test --test upscale real_nvenc_upscale -- --ignored --nocapture
```

These require FFmpeg/FFprobe, FFV1 fixture generation, and the corresponding working encoders. They check controlled 8-bit H.264/H.265 and 10-bit H.265 PQ/HLG output dimensions, bit depth, and color signaling. They do not replace native-window review or quality comparisons.

Before release, smoke-test preset 1440p/4K output and 8K sizing, custom landscape/ultrawide/portrait boxes, both NVENC codecs at CQ 16 and custom CQ, Auto success/fallback, explicit unavailable NVENC, and supported/unsupported video formats. Compare a controlled source using the previous CPU defaults, the new equivalent CPU settings, and NVENC CQ 16 for regression sanity, not to claim one encoder is universally better. Record actual results and unavailable cases separately.

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

Or omit `--zip` to select the newest `<project-slug>*.zip` from Downloads. Automatic discovery accepts both full project snapshots and changed-files packages, including browser duplicate names such as ` (1)` and ` (2)`. Use `--pattern` to narrow discovery when needed. The importer rejects absolute/traversal paths and `.git`, refuses tracked dirty state by default, skips identical files, supports dry run, requires explicit opt-in for directory deletion, and backs up overwritten/deleted paths by default under:

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

# Apollo Testing

> [!IMPORTANT]
> All of the documentation here are written by LLMs.

Use the smallest check that proves the change works. Windows x64 is the supported platform and the target of the CI workflows. A Linux check can catch Rust regressions but does not establish native Windows behavior.

## Local verification

Run commands from the repository root with the pinned Rust toolchain. See [Development](DEVELOPMENT.md) for setup.

| Command | Purpose |
| --- | --- |
| `just fmt-check` | Check formatting without changing files. |
| `just lint` | Run Clippy on all targets and features with warnings denied. |
| `just check` | Check compilation. |
| `just test` | Run all non-ignored Rust tests across all targets. |
| `just build` | Build the application. |
| `just check-release` | Validate version metadata without modifying files. |
| `just verify` | Run formatting, lint, tests, build and metadata validation in order; stop at the first failure. |
| `just test-scripts` | Optional local PowerShell tooling smoke check. |

CI runs Rust formatting, Clippy and tests on Windows. Windows build and release workflows also validate the actual artifacts. The optional tooling smoke runner is independent of CI, `just verify` and release packaging. The local gate does not build an installer or run GPU acceptance.

## Test locations

Unit tests live alongside the Rust implementation. Integration tests live under `tests/`.

| Test target | Coverage |
| --- | --- |
| `audio_mix` | Track selection, mixing and command construction. |
| `upscale` | Sizing, quality, backend resolution and format policy, plus opt-in real encodes. |
| `remux` | Stream-copy mapping and compatibility. |
| `audio_extract` | Selected-stream validation and output formats. |
| `frame_capture` | Frame selection and image output requests. |
| `media_probe` | FFprobe fixture parsing and shared media models. |
| `bundled_media_tools` | Media-tool discovery and installed/portable acceptance harness. |

Queue state, intake identity, output publication, process lifecycle, UI state and settings also have inline tests. Choose the module or integration target that owns the behavior.

```powershell
cargo test --locked --test remux
cargo test --locked --lib queue::
cargo test --locked --bin apollo app::tests::
```

Add a name filter to narrow a test run further. Use existing fixtures where possible. Test observable outcomes and failure handling, not exact error text, incidental implementation details or duplicated assertions. Prefer explicit completion conditions with bounded waits over fixed sleeps. Use disposable media and temporary directories; never use the user's media library as a test fixture.

For concurrency tests, hold work at a controlled boundary and release it explicitly. Check completion, identity and ordering, not the time a worker takes to run. For command construction, assert external command semantics and connected filter stages rather than incidental labels or whole-string snapshots. Remove tests that duplicate coverage without protecting a distinct behavior.

For filesystem and processing changes, verify that cancellation, output collisions, failed processes and stale results preserve existing files and cannot publish an incomplete result. For queue changes, cover job transitions, destination reservations and atomic batch insertion at their owning layer.

## Real media tests

The normal Rust gate covers sizing, encoder resolution, quality bounds, command construction, and format validation without requiring GPU hardware. Opt-in tests exercise real CPU/NVENC encoding and FFprobe verification:

```powershell
cargo test --locked --test upscale real_cpu_upscale -- --ignored --nocapture
cargo test --locked --test upscale real_nvenc_upscale -- --ignored --nocapture
```

These require FFmpeg/FFprobe, FFV1 fixture generation, and the corresponding working encoders. They check controlled 8-bit H.264/H.265 and 10-bit H.265 PQ/HLG output dimensions, bit depth, and color signaling. They do not replace native-window review or quality comparisons.

For the real Windows tool-discovery test, stage the pinned pair as described in [Development](DEVELOPMENT.md#pinned-media-tools-for-development):

```powershell
$env:APOLLO_TEST_MEDIA_TOOLS_DIR = (Resolve-Path .scratch/dev-media).Path
cargo test --locked --test bundled_media_tools real_pair_ -- --ignored --nocapture
```

Set PATH to the staged media directory in the shell used for the real Upscale tests. The GPU tests require working NVIDIA encoders; compiled encoder inventory alone does not prove a successful encode.

## Native interface checks

Run `just run` on Windows when changing interaction, native dialogs, clipboard behavior or layout. Check affected views at narrow, default and wide window sizes. Confirm applicable loading, empty, failure, cancellation and success states, keyboard access and focus. Rust state tests cannot establish the rendered layout or native interaction.

For queue changes, exercise empty and mixed-state queues, each job state and long paths at the affected window sizes. Check the aligned Queue header and columns, stacked narrow rows, disabled reorder-arrow legibility, active-`JobId` progress, and separation of Back from process controls.

Also verify both queue-edit entry points with same-source duplicate jobs and different sources; Save and Cancel return to the correct origin; changing operation preserves the queued source; retained queued media opens without FFprobe; and successful single-item enqueue keeps its source workspace with `Added to queue` feedback and an updated pending count.

For batch changes, exercise native multi-select, multi-file drop and Add files. Check duplicate detection, shared defaults, Ready Override, Needs attention Review, inspection failures, Include/Exclude and atomic enqueue. Confirm that appending preserves existing state, inspection never exceeds two concurrent probes, and failed or incompatible sources expose no inert actions.

Exercise duplicate queued destinations, genuine existing output files, invalid requests, changed or missing sources, and short and long diagnostics. Concise corrective guidance must remain visible without opening Details, while expanded diagnostics use only the required height up to the shared maximum.

For Upscale changes, exercise preset and custom sizing, CPU and both NVENC codecs where available, Auto fallback, explicit unavailable NVENC and supported or unsupported pixel formats. Check output dimensions and color metadata with FFprobe. Inspect representative output for visual regressions; metadata alone does not establish visual fidelity or full HDR preservation.

## Windows artifact acceptance

Run against the exact artifacts being reviewed, with the checkout and built `Apollo.exe` matching their revision. The script checks artifact sums, compares every payload byte to the build and a freshly verified offline archive extraction, and verifies installed metadata. It refuses to touch an existing Apollo registration or shortcut, so use a clean Windows user or VM if Apollo is already installed.

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

For both portable and installed payloads, the harness uses Apollo's production discovery, probe, feature executors and output publication with a one-second 320x180/two-audio-track source:

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

### NVIDIA baseline

The [build acceptance record](../distribution/media-tools-acceptance.md) identifies the tested baseline: driver 566.14 on an RTX 3060 Laptop GPU, with `ffnvcodec n12.2.72.0-1-g9934f17` defining NVENC API 12.2. This is not the oldest supported driver measurement and does not certify other GPUs. Encoder inventory and `ffmpeg -version` alone cannot establish this baseline.

On that baseline, rerun the same artifact check with mandatory GPU success:

```powershell
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
pwsh -NoProfile -File .\tests\scripts\WindowsRelease.Smoke.ps1 `
    -ReleaseDirectory artifacts/windows -DiscoveryHarness $harness[0].executable -RequireNvenc
if ($LASTEXITCODE -ne 0) { throw 'NVIDIA release acceptance failed.' }
```

This adds both successful real NVENC probes, explicit H.264/H.265 Upscale at CQ 16, and Auto choosing NVENC for each codec. Every output is probed for codec/dimensions, and process failure is fatal. Confirm there is no required-NVENC-API-newer-than-driver failure. If a deliberately incompatible-driver machine is available, also record its explicit NVENC diagnostic and Auto CPU result; do not alter a user's driver merely to obtain that evidence.

GPU success is a manual publication gate. GitHub-hosted CI does not require it. Rebuilt artifacts need new acceptance even when the FFmpeg pin is unchanged.

For pin changes, repeat the relevant checks in [the media-tool acceptance record](../distribution/media-tools-acceptance.md) and review redistribution obligations for the exact binaries. Retain the required license and notice material. Artifact tests do not establish corresponding-source compliance by themselves.

## Optional tooling smoke check

`just test-scripts` runs `tests/scripts/run.ps1` in disposable repositories. It checks PowerShell syntax, snapshot export, review, change transfer and rollback, unsafe archive rejection and cleanup protection. Keep this runner small and local-only; it must not gate application CI or packaging.

## Reporting results

Record the commands, platform, actual results and any skipped or unavailable cases. For artifact acceptance, record the date, revision and local changes, Windows build, artifact and executable hashes, and media-tool manifest hash; include GPU and driver information for NVIDIA checks. Do not report an ignored test, simulated process or an earlier artifact's results as current runtime acceptance.

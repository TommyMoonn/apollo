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

CI runs Rust formatting, strict Clippy, locked tests, a locked application build and read-only metadata validation on Windows. The aggregate `CI Gate` requires all four validation jobs to succeed; see [Continuous integration](DEVELOPMENT.md#continuous-integration) for the job and branch-protection contract. Windows build and release workflows separately validate the actual artifacts. The optional tooling smoke runner is independent of CI, `just verify` and release packaging. The local gate does not build an installer or run GPU acceptance.

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

Queue state, intake identity, output publication, process lifecycle, UI state and settings also have unit tests. The application core has renderer-independent navigation, media-tool discovery and source-session tests. Source inspection tests use completion notifications and existing FFprobe fixtures; the legacy adapter tests retain source replacement, picker cancellation, drag/drop and draft retention coverage. The legacy adapter tests cover editor return flows, root update behavior and pending encoder capability presentation. Choose the module or integration target that owns the behavior.

```powershell
cargo test --locked --test remux
cargo test --locked --lib queue::
cargo test --locked --lib app::
cargo test --locked --lib ui::legacy::tests::
```

Add a name filter to narrow a test run further. Use existing fixtures where possible. Test observable outcomes and failure handling, not exact error text, incidental implementation details or duplicated assertions. Prefer explicit completion conditions with bounded waits over fixed sleeps. Use disposable media and temporary directories; never use the user's media library as a test fixture.

For concurrency tests, hold work at a controlled boundary and release it explicitly. Use completion callbacks to verify results are available before notification without requiring a renderer. Check completion, identity and ordering, not the time a worker takes to run. For command construction, assert external command semantics and connected filter stages rather than incidental labels or whole-string snapshots. Remove tests that duplicate coverage without protecting a distinct behavior.

For application and UI tests, drive lifecycle events with explicit timestamps or completion signals. Check interaction, selection, focus, clipping, non-overlap and reachability at narrow and normal widths; avoid exact paint snapshots, copied theme constants and incidental pixel geometry. Retain renderer-specific regressions until their replacement proves the same behavior.

For filesystem and processing changes, verify that cancellation, output collisions, failed processes and stale results preserve existing files and cannot publish an incomplete result. For queue changes, cover job transitions, destination reservations and atomic batch insertion at their owning layer.

## Real media tests

The normal Rust gate covers sizing, encoder resolution, quality bounds, command construction, and format validation without requiring GPU hardware. Opt-in tests exercise real CPU/NVENC encoding and FFprobe verification:

```powershell
cargo test --locked --test upscale real_cpu_upscale -- --ignored --nocapture
cargo test --locked --test upscale real_nvenc_upscale -- --ignored --nocapture
```

These require FFmpeg/FFprobe, FFV1 fixture generation, and the corresponding working encoders. They check controlled 8-bit H.264/H.265 and 10-bit H.265 PQ/HLG output dimensions, bit depth, and color signaling. They do not replace native-window review or quality comparisons.

The discovery subprocess is an ignored harness entry point, not a standalone passing test. The Windows discovery and release acceptance callers select it explicitly and verify that the scenario completes.

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

Use a clean Windows x64 user or VM, PowerShell 7, the pinned Rust toolchain, and an authenticated GitHub CLI with read access to Apollo's public releases. Run from the checkout matching the staged artifacts. Neither the original application build nor the cached FFmpeg archive is needed.

```powershell
just smoke-installer --local --expected-version 0.7.0
```

For a downloaded CI artifact, use its recorded version and SHA with `just smoke-installer --artifacts-dir <path> --expected-sha <sha> --expected-version X.Y.Z`. The command independently verifies the artifact set before installation and builds the existing discovery harness once. Use `--discovery-harness <path>` to reuse a harness already built from the matching checkout. Pass `--upgrade-only` to require a strictly older published release.

The smoke run refuses existing Apollo registration, shortcuts, the default installation directory or a running Apollo process. It uses a unique `.scratch/release-smoke-*` directory, verifies installed file digests against the portable payload, and runs the same CPU and unavailable-NVIDIA cases against each distribution. It checks installed version, registration, shortcut targets, uninstall ownership and preservation of unrelated files. PATH is cleared only for child processes.

After clean installation acceptance, the script downloads the official prior published installer and its checksum from GitHub. It installs that version in another isolated directory, creates unrelated files, runs the candidate without an installation-directory override, and checks the resulting version, location, payload, registration, previously enabled desktop shortcut and retained file contents before uninstalling. For a manual build of the currently published version, the test is a same-version reinstall. If no checksum-verified predecessor is available, upgrade acceptance fails with an explicit blocked reason; it does not substitute an unrelated installer. Scratch files are removed after the run; if ownership of unexpected registration cannot be established, cleanup refuses to delete it and retains the scratch directory for investigation.

For media-only acceptance without an installation or release artifact, stage the pinned pair and run:

```powershell
just test-media
just test-media --media-dir 'C:/Media Tools' --require-nvenc
```

The default pair location is `.scratch/dev-media`. The command copies the selected pair into disposable scratch space and uses the same production-executor harness. It does not modify the original media-tool directory, download a predecessor or touch installer registration. Generated fixtures are removed after the check. Both commands are outside `just verify`.

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

On that baseline, rerun the same artifact check with mandatory GPU success, substituting the actual version and artifact identity:

```powershell
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
just smoke-installer --local --expected-version 0.7.0 --require-nvenc
```

This adds both successful real NVENC probes, explicit H.264/H.265 Upscale at CQ 16, and Auto choosing NVENC for each codec. Every output is probed for codec/dimensions, and process failure is fatal. Confirm there is no required-NVENC-API-newer-than-driver failure. If a deliberately incompatible-driver machine is available, also record its explicit NVENC diagnostic and Auto CPU result; do not alter a user's driver merely to obtain that evidence.

Complete applicable manual GPU acceptance before approving the release preparation PR, and record any unavailable hardware or unresolved limitations for owner review. Neither hosted CI nor automatic publication verifies successful NVENC on real hardware. A pre-merge build provides acceptance evidence for its own bytes only; repeat the hardware check on rebuilt artifacts when verifying the published distribution.

For pin changes, repeat the relevant checks in [the media-tool acceptance record](../distribution/media-tools-acceptance.md) and review redistribution obligations for the exact binaries. Retain the required license and notice material. Artifact tests do not establish corresponding-source compliance by themselves.

## Optional tooling smoke check

`just test-scripts` runs `tests/scripts/run.ps1` in disposable repositories. It checks PowerShell syntax, snapshot export, review, change transfer and rollback, unsafe archive rejection and cleanup protection. Keep this runner small and local-only; it must not gate application CI or packaging.

## Reporting results

Record the commands, platform, actual results and any skipped or unavailable cases. For artifact acceptance, record the date, revision and local changes, Windows build, artifact and executable hashes, and media-tool manifest hash; include GPU and driver information for NVIDIA checks. Do not report an ignored test, simulated process or an earlier artifact's results as current runtime acceptance.

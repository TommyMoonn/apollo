# Changelog

## [Unreleased]

## [0.4.0] - 2026-09-09

### Added

- Custom Upscale width and height boundaries alongside 1440p, 4K, and 8K presets, with a calculated output preview that preserves the source aspect ratio.
- Auto, CPU, and NVIDIA NVENC encoder selection for H.264 and H.265. Auto uses NVENC when its runtime check succeeds, otherwise CPU; explicit unavailable NVENC shows the failure reason instead of silently switching encoders.
- Numeric Upscale quality controls, with separate values retained for each codec and encoder backend.

### Changed

- Upscale explicitly preserves supported pixel formats and available color signaling, including supported 10-bit H.265 paths. Unsupported format combinations are rejected rather than silently reduced to 8-bit. Full HDR metadata preservation is not guaranteed.

## [0.3.2] - 2026-09-01

Apollo 0.3.2 makes file selection and normal window interaction more responsive, and adds visual confirmation for completed frame captures.

### Changed

- Native Open and Browse dialogs now reuse one persistent service, reopen without whole-window dimming, and remember source and output directories separately.
- Idle intake and loaded-source workspaces no longer run periodic Apollo repaint loops or repeat media and filesystem work solely because the window repainted.
- Running operations now keep progress readable while bounding rapid visual updates, with cancellation and terminal results still appearing immediately.
- Successful Frame Capture results now show an aspect-ratio-preserving preview while keeping the published full-resolution image unchanged. Preview failures do not invalidate a successful capture.

## [0.3.1] - 2026-09-01

Apollo 0.3.1 reorganizes the existing media tools into a persistent source workspace with direct operation switching and consistent controls.

### Changed

- A loaded source now stays visible and active while switching between Audio Mix, Upscale, Remux, Audio Extract, and Frame Capture.
- `Change file` replaces the current source only after the new file is selected and inspected, while `Close source` explicitly returns to the empty intake screen.
- Every operation now shares the same source header, operation switcher, scrollable configuration area, and bottom-anchored primary action.
- Output controls now use a consistent selector, editable filename, extension, and Browse layout across all five operations.
- Operation changes, conditional controls, and button hover states now use short restrained fades without moving or resizing controls.

### Fixed

- Cancelling or failing a source replacement keeps the current workspace intact and now leaves replacement errors visible until the source lifecycle changes.
- The selected operation remains visually distinct while workspace controls are temporarily disabled.
- Button hover shading now fades smoothly both when the pointer enters and when it leaves.

## [0.3.0] - 2026-08-31

Apollo 0.3.0 adds Remux, Audio Extract, and Frame Capture as focused media utilities alongside Audio Mix and Video Upscale.

### Added

- Remux can copy every compatible source stream into MKV or MP4 without re-encoding, while preserving source metadata and chapters.
- Audio Extract can save one selected audio track as an original MKA copy, 256 kbps AAC, or lossless FLAC.
- Frame Capture can save one timestamped still image as lossless PNG or high-quality JPEG.

### Changed

- Remux now identifies the first incompatible stream before processing and recommends another supported container only when the complete source is compatible with it.
- Audio Extract preserves known track titles and languages, and generated names identify the selected stream when a source has multiple audio tracks.
- Frame Capture generated names include the selected timestamp, known source duration is shown beside the timestamp field, and captures at or beyond the source end are rejected.

### Fixed

- Frame Capture now treats an empty FFmpeg result as a failure instead of publishing a misleading successful output.

## [0.2.0] - 2026-08-30

Apollo 0.2.0 adds user-controlled Audio Mix processing with per-track gain, optional loudness normalization, stable mix levels, and deterministic mixed-audio output.

### Added

- Every selected Audio Mix track now has an independent gain control from -60.0 dB to +12.0 dB, with stored values preserved when tracks are deselected or reselected.
- Optional output loudness normalization now exposes configurable integrated loudness, maximum true peak, and loudness range targets.
- Audio Mix now includes dedicated controls for resetting track gains and normalization targets without changing the selected tracks or output settings.

### Changed

- Audio Mix no longer reduces every selected track based on the number of inputs; configured gains remain stable and non-normalized mixes receive final peak protection.
- Mixed audio now uses an explicit mono or stereo layout independent of selected-track order, with unsupported wider layouts reported before processing.
- Mixed tracks are encoded consistently as AAC at 320 kbps and 48 kHz and receive the title `Mixed Audio` in MP4 and MKV outputs.
- The Audio Mix ready surface now groups track selection, gain, loudness normalization, and output controls into clearer aligned sections.

## [0.1.2] - 2026-08-29

Apollo 0.1.2 is a corrective Windows release that improves interface responsiveness, removes terminal-backed file selection, and makes installed Apollo builds easier to discover and manage.

### Changed

- Media-tool discovery and media inspection now initialize in the background and wake the interface when results are ready instead of blocking normal window interaction.
- Destination filesystem checks now run at execution preflight rather than during ready-surface rendering, while existing output-safety rules remain authoritative.
- The Windows installer now uses a conventional Welcome, License, install-location, component, install, and Finish flow with an optional desktop shortcut and direct post-install launch.
- Installed builds now register Apollo through the current-user Start Menu, App Paths, and Installed Apps entries while preserving the established per-user upgrade and uninstall identity.
- Windows release executables now expose Cargo-derived file and product version metadata alongside the Apollo application icon.

### Fixed

- Open and Browse now use Apollo-owned native Windows file dialogs without PowerShell or WinForms startup, and stale dialog results cannot change a replacement or running tool session.
- FFmpeg and FFprobe background processes no longer open visible console windows during discovery, inspection, Audio Mix, or Video Upscale work.
- Ready and intake surfaces avoid repeated presentation and layout work when their underlying media or error state has not changed.

## [0.1.1] - 2026-08-29

Apollo 0.1.1 is a corrective Windows release that improves standalone startup, external media-tool detection, and application branding.

### Changed

- Apollo now uses its own application icon in native windows, the portable executable, the installer and uninstaller, Start Menu shortcuts, and Installed Apps.

### Fixed

- Installed and portable builds now carry the required IBM Plex Sans and Fira Code fonts inside `Apollo.exe`, so startup no longer depends on font files from the source repository.
- Existing system FFmpeg and FFprobe installations are now discovered using native Windows executable-search behavior, with resolution, launch, timeout, and validation failures reported distinctly.

## [0.1.0] - 2026-08-28

Apollo 0.1.0 is the first public Windows release, bringing focused Audio Mix and Video Upscale workflows in a compact native interface.

### Added

- Audio Mix for selecting embedded audio tracks, mixing the selection into one AAC stream, preserving the source video without re-encoding, and writing MP4 or MKV output.
- Video Upscale with 1440p, 4K, and 8K targets, aspect-ratio-preserving Lanczos scaling, H.264 or H.265 output, and compatible audio preservation without unnecessary re-encoding.
- Editable output filenames and native destination browsing for both workflows, with required output extensions kept consistent with the selected format.
- Native progress, cancellation, completion, and failure states for both media operations.
- A per-user Windows installer that creates a Start Menu shortcut and Installed Apps entry without requiring elevation.
- A portable Windows ZIP for users who prefer to run Apollo without installing it.

### Changed

- The interface adapts ready surfaces, option lists, output rows, and actions across narrow, normal, wide, tall, and fullscreen layouts.
- Media inspection runs without blocking the interface and shows an `Inspecting media…` state while the selected tool can still be changed.
- Drag and drop accepts one source at a time and reports multi-file drops instead of silently processing only the first file.
- Apollo is distributed under the GNU General Public License v3.0 only, with third-party notices included in release packages.

### Fixed

- Outputs are staged safely and published only after successful FFmpeg completion; failed or cancelled operations do not leave a misleading completed file.
- Output validation catches same-source destinations, unavailable output folders, existing output files, unusable filenames, and Audio Mix format-extension mismatches before processing starts.
- Windows output paths keep their normal user-selected representation through staging and completion instead of leaking canonical verbatim path forms.
- Missing optional FFprobe metadata falls back to usable media and track labels instead of preventing intake or selection.
- Upscale sizing preserves the source aspect ratio within the selected target boundary without default cropping, stretching, or black padding.

[Unreleased]: https://github.com/TommyMoonn/apollo/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/TommyMoonn/apollo/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/TommyMoonn/apollo/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/TommyMoonn/apollo/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/TommyMoonn/apollo/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/TommyMoonn/apollo/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/TommyMoonn/apollo/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/TommyMoonn/apollo/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/TommyMoonn/apollo/releases/tag/v0.1.0

# Changelog

## [Unreleased]

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

[Unreleased]: https://github.com/TommyMoonn/apollo/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/TommyMoonn/apollo/releases/tag/v0.1.0

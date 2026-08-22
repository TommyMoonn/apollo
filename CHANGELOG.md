# Changelog

## [Unreleased]

### Added

- Audio Mix workflow for selecting embedded audio tracks, mixing the selection into one AAC stream, preserving the source video without re-encoding, and writing MP4 or MKV output.
- Video Upscale workflow with 1440p, 4K, and 8K targets, aspect-ratio-preserving Lanczos scaling, H.264 or H.265 video output, and compatible audio preservation without unnecessary re-encoding.
- Editable output filenames and native destination browsing for both workflows, with required output extensions kept in sync with the selected container.
- Native progress, cancellation, completion, and failure states for both media operations.

### Changed

- The native interface adapts ready surfaces, option lists, output rows, and action groups across narrow, normal, wide, tall, and fullscreen layouts.
- Media inspection runs outside the egui frame path and shows an `Inspecting media…` state while mode switching remains available.
- File drag-and-drop is explicitly single-source: one dropped file enters normal intake, while multi-file drops are rejected instead of silently processing only the first file.

### Fixed

- Final outputs are staged in temporary owned locations and published only after successful FFmpeg completion; failed and cancelled operations discard temporary output instead of leaving a misleading final file.
- Output validation rejects same-source destinations, unavailable output folders, existing output files, unusable filenames, and Audio Mix container-extension mismatches before media execution starts.
- Missing optional FFprobe metadata degrades to usable media and track fallbacks instead of preventing intake or selection.
- Upscale sizing preserves the source aspect ratio within the selected target boundary without default crop, stretch, or black padding.

### Testing

- Added regression coverage for FFprobe metadata fallbacks, Audio Mix stream selection and mapping, Upscale sizing and codec/audio policies, output publication safety, cancellation/failure lifecycle, and responsive or disabled UI states.

<p align="center">
  <img src="docs/assets/apollo-wordmark.png" alt="Apollo" width="240" />
</p>

<p align="center">
  A native Windows utility for focused media workflows.
</p>

<p align="center">
  <a href="#download-apollo">Download</a>
  ·
  <a href="#features">Features</a>
  ·
  <a href="#requirements">Requirements</a>
  ·
  <a href="#project-documentation">Documentation</a>
  ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

<p align="center">
  <img src="docs/assets/apollo-preview.png" alt="Apollo file intake screen" width="620" />
</p>

## About Apollo

Apollo is a small native desktop media utility for audio mixing, video upscaling, stream-copy remuxing, audio extraction, and timestamped frame capture.

## Download Apollo

- **[Windows EXE installer](https://github.com/TommyMoonn/apollo/releases/latest/download/Apollo-Setup-x64.exe)** (recommended)
- **[Portable Windows ZIP](https://github.com/TommyMoonn/apollo/releases/latest/download/Apollo-windows-x64.zip)**

The installer registers Apollo for the current Windows user, creates a Start Menu shortcut, and provides normal Windows uninstall behavior. The portable ZIP does not register or install Apollo.

See the [latest release notes](https://github.com/TommyMoonn/apollo/releases/latest) or browse [all releases](https://github.com/TommyMoonn/apollo/releases).

## Features

- **Audio Mix** - choose the embedded audio tracks to keep, mix them into one AAC stream, and write MP4 or MKV output without re-encoding the video.
- **Video Upscale** - upscale to 1440p, 4K, or 8K boundaries with Lanczos scaling and H.264 or H.265 output while preserving the source aspect ratio.
- **Remux** - copy all compatible streams into MKV or MP4 without re-encoding, with incompatibilities reported before processing.
- **Audio Extract** - save one selected audio track as an original MKA copy, 256 kbps AAC, or lossless FLAC.
- **Frame Capture** - save one timestamped frame as lossless PNG or high-quality JPEG.
- **Output control** - edit the output filename and choose the destination before starting any workflow.
- **Safe publication** - work is written to an owned temporary location and published only after successful completion; failed or cancelled work does not leave a misleading final file.
- **Responsive native interface** - the compact egui interface adapts across narrow, normal, wide, tall, and fullscreen windows.

## Requirements

- Windows x64.
- FFmpeg and FFprobe available on `PATH`.
- For Upscale, the installed FFmpeg build must provide the selected `libx264` or `libx265` encoder.

FFmpeg and FFprobe are external runtime requirements and are not bundled with either Apollo download.

## Native and focused

Apollo keeps the interface native and the workflow direct. The application owns media selection, validation, progress, cancellation, and safe output publication, while FFmpeg and FFprobe perform media inspection and processing.

## Project documentation

- [Development guide](docs/DEVELOPMENT.md)
- [Release process](docs/RELEASES.md)
- [Project scripts](scripts/README.md)
- [Changelog](CHANGELOG.md)

## License

Apollo is licensed under the [GNU General Public License v3.0 only](LICENSE).

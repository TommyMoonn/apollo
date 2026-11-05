<p align="center">
  <img src="docs/assets/apollo-wordmark.png" alt="Apollo" width="240" />
</p>

<p align="center">
  A native Windows media utility.
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

# About Apollo

I made this for the random clips I record now and then, it handles audio mixing, video upscaling, remuxing, audio extraction, and frame capture.

This is entirely vibecoded and as of currently I have no goal to continue developing or maintaining this project.

> [!IMPORTANT]
> All of the stuff under here are written by LLMs.

## Download Apollo

- **[Windows installer](https://github.com/TommyMoonn/apollo/releases/latest/download/Apollo-Setup-x64.exe)** (recommended)
- **[Portable Windows ZIP](https://github.com/TommyMoonn/apollo/releases/latest/download/Apollo-windows-x64.zip)**

The installer adds Apollo to the Start Menu and Windows uninstall list. The portable ZIP runs without installation.

See the [latest release notes](https://github.com/TommyMoonn/apollo/releases/latest) or browse [all releases](https://github.com/TommyMoonn/apollo/releases).

## Features

- **Audio mixing:** Select embedded audio tracks and mix them into one AAC stream while keeping the video unchanged. Output MP4 or MKV.
- **Video upscaling:** Fit video within 1440p, 4K, 8K, or custom dimensions while preserving its aspect ratio. Use H.264 or H.265 with CPU or NVIDIA encoding.
- **Remuxing:** Copy compatible streams into MP4 or MKV without re-encoding.
- **Audio extraction:** Export a selected audio track as MKA, AAC, or FLAC.
- **Frame capture:** Save a frame at a timestamp as PNG or JPEG.
- **Queue and batch processing:** Queue jobs, reorder or edit waiting jobs, and process batches with shared settings and per-file overrides.
- **Output handling:** Choose the output filename and destination. Apollo publishes the final file only after processing succeeds.

## Requirements

Apollo supports Windows x64. CPU H.264 and H.265 encoders are included. NVIDIA encoding is optional and requires a compatible GPU and driver.

See the [pinned media-tool manifest](distribution/media-tools.json), [compatibility notes](distribution/media-tools-acceptance.md), and [third-party notices](THIRD_PARTY_NOTICES.md#bundled-ffmpeg-and-ffprobe).

## Project documentation

- [Development guide](docs/DEVELOPMENT.md)
- [Release process](docs/RELEASES.md)
- [Project scripts](scripts/README.md)
- [Changelog](CHANGELOG.md)

## License

Apollo is licensed under the [GNU General Public License v3.0 only](LICENSE).

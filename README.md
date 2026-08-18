# Apollo

Apollo is a lightweight native desktop media utility written in Rust.

## Project identity

- Application name: **Apollo**
- Repository slug: `apollo`
- Release line: `0.1.0`

## Current scope

Apollo currently has its native `egui`/`eframe` application shell, repository workflow tooling, shared FFmpeg/FFprobe discovery, reusable FFprobe media inspection, a shared long-running media process lifecycle, safe temporary output publication, and machine-readable FFmpeg progress parsing. FFmpeg and FFprobe are resolved from `PATH` and validated before the media tools are made available. Audio mixing and upscaling remain deferred to later phases.

## Toolchain

The project is pinned to Rust `1.97.1` and uses the Rust 2024 edition.

```powershell
cargo fmt --check
cargo check
cargo test
```

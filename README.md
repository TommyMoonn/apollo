# Apollo

Apollo is a lightweight native desktop media utility written in Rust.

## Project identity

- Application name: **Apollo**
- Repository slug: `apollo`
- Release line: `0.1.0`

## Current scope

Apollo currently has its native `egui`/`eframe` application shell, repository workflow tooling, and shared FFmpeg/FFprobe discovery. FFmpeg and FFprobe are resolved from `PATH` and validated before the media tools are made available. Media probing and processing remain deferred to later phases.

## Toolchain

The project is pinned to Rust `1.97.1` and uses the Rust 2024 edition.

```powershell
cargo fmt --check
cargo check
cargo test
```

# Apollo

Apollo is a lightweight native desktop media utility written in Rust.

## Project identity

- Application name: **Apollo**
- Repository slug: `apollo`
- Release line: `0.1.0`

## Foundation scope

The repository currently contains the project bootstrap, development rules, and native `egui`/`eframe` application shell from Phases `0.1.0.1` through `0.1.0.4`. The shared PowerShell CLI parser and its focused script tests are also established. Media behavior and FFmpeg integration remain intentionally deferred to later phases.

## Toolchain

The project is pinned to Rust `1.97.1` and uses the Rust 2024 edition.

```powershell
cargo fmt --check
cargo check
cargo test
```

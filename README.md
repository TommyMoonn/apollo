# Apollo

Apollo is a lightweight native desktop media utility written in Rust.

## Project identity

- Application name: **Apollo**
- Repository slug: `apollo`
- Release line: `0.1.0`

## Foundation scope

The repository currently contains the project bootstrap and development rules from Phases `0.1.0.1` and `0.1.0.2`. Media behavior and the native `egui`/`eframe` application shell are intentionally deferred to later phases.

## Toolchain

The project is pinned to Rust `1.97.1` and uses the Rust 2024 edition.

```powershell
cargo fmt --check
cargo check
cargo test
```

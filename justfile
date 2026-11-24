set shell := ["pwsh", "-NoLogo", "-NoProfile", "-Command"]

# List available commands.
default:
    @just --list

# Run the application using the pinned toolchain.
[group("development")]
run:
    cargo run --locked

# Build the application.
[group("development")]
build:
    cargo build --locked

# Check compilation without producing a binary.
[group("development")]
check:
    cargo check --locked

# Format Rust source.
[group("verification")]
fmt:
    cargo fmt

# Check Rust formatting without writing files.
[group("verification")]
fmt-check:
    cargo fmt --check

# Run Clippy across supported targets and features; deny warnings.
[group("verification")]
lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings

# Run the Rust tests.
[group("verification")]
test:
    cargo test --locked --all-targets

# Generate optional Rust coverage diagnostics.
[group("verification")]
coverage:
    pwsh -NoProfile -File scripts/coverage-rust.ps1

# Optional local smoke check of repository tooling.
[group("verification")]
test-scripts:
    pwsh -NoProfile -File tests/scripts/run.ps1

# Run all local checks once, stopping on the first failure.
[group("verification")]
verify: fmt-check lint test build check-release

# Export repo or workspace; optional --project and --output.
[group("workspace")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
zip mode *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/zip-project.ps1 --mode @args

# Review working-tree changes; use --files for paths.
[group("workspace")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
review *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/review-changes.ps1 @args

# Package a named change set; optional --tracked-only.
[group("workspace")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
package name="changes" *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/package-changes.ps1 --name @args

# Apply a changed-files ZIP; optional --zip and --dry-run.
[group("workspace")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
apply *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/apply-changes.ps1 @args

# Restore an import backup; optional --backup and --dry-run.
[group("workspace")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
restore *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/restore-changes.ps1 @args

# Clean Rust target/; use --dry-run to preview or --force for tracked content.
[group("workspace")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
clean *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/clean-generated.ps1 @args

# Update release version metadata.
[group("release")]
set-version version:
    pwsh -NoProfile -File .\scripts\set-version.ps1 {{version}}

# Validate version metadata without modifying files.
[group("release")]
check-release:
    pwsh -NoProfile -File scripts/check-release.ps1

# Validate a release tag and changelog.
[group("release")]
check-release-tag tag:
    pwsh -NoProfile -File .\scripts\check-release.ps1 --tag {{tag}} --release-changelog

# Build the Windows release executable.
[group("release")]
build-release:
    cargo build --locked --release --target x86_64-pc-windows-msvc

# Build the Windows installer.
[group("release")]
build-installer:
    pwsh -NoProfile -File .\scripts\build-windows-installer.ps1

# Stage Windows release artifacts.
[group("release")]
stage-release:
    pwsh -NoProfile -File .\scripts\stage-windows-release.ps1

# Validate metadata for a specified release version.
[group("release")]
release-check version:
    pwsh -NoProfile -File .\scripts\check-release.ps1 --tag v{{version}} --release-changelog

# Build and stage the Windows release.
[group("release")]
release-stage:
    cargo build --locked --release --target x86_64-pc-windows-msvc
    pwsh -NoProfile -File .\scripts\stage-windows-release.ps1

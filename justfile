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
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
set-version version *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/set-version.ps1 @args

# Validate version metadata; optional --tag and --require-changelog.
[group("release")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
check-release *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/check-release.ps1 @args

# Build the Windows release executable.
[group("release")]
build-release:
    cargo build --locked --release --target x86_64-pc-windows-msvc

# Stage the pinned media payload without building Apollo.
[group("release")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
stage-media-tools *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/stage-media-tools.ps1 @args

# Build the Windows installer from an existing executable.
[group("release")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
build-installer *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/build-windows-installer.ps1 @args

# Stage Windows release artifacts without rebuilding Apollo.
[group("release")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
stage-release *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/stage-windows-release.ps1 @args

# Verify a staged or downloaded Windows artifact set.
[group("release")]
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
verify-release *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/verify-windows-release.ps1 @args

# Build and stage the Windows release.
[group("release")]
release-stage:
    cargo build --locked --release --target x86_64-pc-windows-msvc
    pwsh -NoProfile -File .\scripts\stage-windows-release.ps1

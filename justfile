set shell := ["pwsh", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

run:
    cargo run

build:
    cargo build

check:
    cargo check

test:
    cargo test --all-targets

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

lint:
    cargo clippy --all-targets --all-features -- -D warnings

gate:
    cargo fmt --check
    cargo clippy --locked --all-targets --all-features -- -D warnings
    cargo test --locked --all-targets

# Forward each argument literally through a PowerShell script recipe.
[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
zip mode *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/zip-project.ps1 --mode @args

[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
review *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/review-changes.ps1 @args

review-files:
    pwsh -NoProfile -File scripts/review-changes.ps1 --files

[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
package name="changes" *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/package-changes.ps1 --name @args

[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
package-tracked name *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/package-changes.ps1 --tracked-only --name @args

[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
apply *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/apply-changes.ps1 @args

[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
apply-file zip *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/apply-changes.ps1 --zip @args

[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
restore *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/restore-changes.ps1 @args

[positional-arguments]
[script("pwsh", "-NoProfile", "-File")]
[extension(".ps1")]
restore-from backup *flags:
    $ErrorActionPreference = 'Stop'
    & ./scripts/restore-changes.ps1 --backup @args

clean:
    pwsh -NoProfile -File scripts/clean-generated.ps1 --rust

clean-dry:
    pwsh -NoProfile -File scripts/clean-generated.ps1 --all --dry-run

clean-all:
    pwsh -NoProfile -File scripts/clean-generated.ps1 --all

test-scripts:
    pwsh -NoProfile -File tests/scripts/run.ps1

test-all:
    cargo test --all-targets
    pwsh -NoProfile -File tests/scripts/run.ps1

set-version version:
    pwsh -NoProfile -File .\scripts\set-version.ps1 {{version}}

check-release:
    pwsh -NoProfile -File .\scripts\check-release.ps1

check-release-tag tag:
    pwsh -NoProfile -File .\scripts\check-release.ps1 --tag {{tag}} --release-changelog

build-release:
    cargo build --locked --release --target x86_64-pc-windows-msvc

build-installer:
    pwsh -NoProfile -File .\scripts\build-windows-installer.ps1

stage-release:
    pwsh -NoProfile -File .\scripts\stage-windows-release.ps1

release-check version:
    pwsh -NoProfile -File .\scripts\check-release.ps1 --tag v{{version}} --release-changelog

release-stage:
    cargo build --locked --release --target x86_64-pc-windows-msvc
    pwsh -NoProfile -File .\scripts\stage-windows-release.ps1

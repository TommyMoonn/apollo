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
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets

zip:
    pwsh -NoProfile -File scripts/zip-project.ps1

review:
    pwsh -NoProfile -File scripts/review-changes.ps1

review-files:
    pwsh -NoProfile -File scripts/review-changes.ps1 --files

package name:
    pwsh -NoProfile -File scripts/package-changes.ps1 --name {{name}}

package-tracked name:
    pwsh -NoProfile -File scripts/package-changes.ps1 --name {{name}} --tracked-only

apply *args:
    pwsh -NoProfile -File scripts/apply-changes.ps1 {{args}}

apply-file zip:
    pwsh -NoProfile -File scripts/apply-changes.ps1 --zip "{{zip}}"

restore:
    pwsh -NoProfile -File scripts/restore-changes.ps1

restore-from backup:
    pwsh -NoProfile -File scripts/restore-changes.ps1 --backup "{{backup}}"

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

vendor:
    pwsh -NoProfile -File .dev/chatgpt/scripts/rust/prepare-chatgpt-cargo-vendor.ps1

toolchain:
    pwsh -NoProfile -File .dev/chatgpt/scripts/rust/prepare-chatgpt-rust-toolchain.ps1

bundles: vendor toolchain

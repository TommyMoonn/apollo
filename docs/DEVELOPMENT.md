# Apollo Development

> [!IMPORTANT]
> All of the documentation here are written by LLMs.

## Prerequisites

Apollo targets Windows as its primary development and validation platform.

Required development tools:

- Git
- Rust toolchain pinned by `rust-toolchain.toml`
- PowerShell 7 for repository scripts
- Just 1.33 or later for development recipes
- FFmpeg and FFprobe for media inspection, processing, and runtime smoke tests

Rustup is the recommended way to install the pinned Rust toolchain. From the repository root, the toolchain file should select the required version automatically.

FFmpeg and FFprobe run as separate processes. The Windows release packages include the pinned pair; `cargo build` does not download or copy media tools. Apollo discovers and validates them through one shared application owner rather than feature-specific checks. Immediate executors and the queue share the media preparation and settlement code; see [Architecture](ARCHITECTURE.md#source-intake-and-execution) for the execution boundaries.

### Pinned media tools for development

`distribution/media-tools.json` owns the exact Windows x64 archive URL, SHA-256, build identity, encoder inventory, redistribution paths and tested compatibility baseline. The pin is FFmpeg 7.0.2, Gyan build `7.0.2-essentials_build-www.gyan.dev`, not an arbitrary 7.0.2 binary.

Stage a verified development pair from the repository root:

```powershell
pwsh -NoProfile -File .\scripts\stage-media-tools.ps1 --output .scratch/dev-media
# A second, new destination can use the existing archive without network access:
pwsh -NoProfile -File .\scripts\stage-media-tools.ps1 --offline --output .scratch/dev-media-offline
```

The default cache is `.scratch/media-tools-cache`; `--cache <path>` selects another cache. A checksum mismatch fails before extraction. Offline cache misses fail without a download. Existing staging destinations are refused. The stager validates both versions and required encoders, but never attempts a GPU encode.

For `just run`, prepend the staged directory to PATH in that development shell, or copy the complete staged payload beside the built Apollo executable. Do not copy just one executable. Apollo prefers a complete valid pair beside the executable. See [Architecture](ARCHITECTURE.md#media-tool-discovery) for discovery and caching rules.

## Local commands

Run `just` or `just --list` from the repository root to discover available recipes. Rust commands use the toolchain pinned in `rust-toolchain.toml` and the checked-in Cargo lockfile.

| Command | Purpose |
| --- | --- |
| `just run` | Start the native application. |
| `just build` | Build the application. |
| `just check` | Check compilation without producing a binary. |
| `just fmt` / `just fmt-check` | Apply or check Rust formatting. |

See [Testing](TESTING.md) for focused tests, `just verify`, optional tooling checks and native Windows acceptance. When changing tests, remove redundant or implementation-coupled cases, preserve meaningful failure coverage and use controlled completion instead of fixed timing assumptions.

## Continuous integration

The `CI` workflow runs the same complete validation for every pull request and every push to `main`. Four independent Windows jobs check formatting, strict Clippy, locked Rust tests followed by a real application build, and read-only release metadata. Lint and test/build use separate Cargo-aware caches keyed by job, Rust compiler and dependency inputs. The test and build steps share a job and Cargo target directory. Formatting and metadata validation do not restore a compilation cache.

The `CI Gate` check succeeds only when all four jobs succeed. It records their results in the run summary, including failed, cancelled or skipped jobs. Superseded pull request runs may be cancelled; each main-push run is kept separate so release validation can find the exact commit's CI result.

`just verify` runs the same checks locally, stopping at the first failure. The optional `just test-scripts` smoke check, coverage diagnostics, `just test-media` and `just smoke-installer` are outside the required gate. See [Testing](TESTING.md#windows-artifact-acceptance) for runtime acceptance and [Releasing](RELEASING.md) for the shared packaging workflow.

For branch protection or rulesets, require the GitHub Actions check named `CI Gate`. First observe a hosted run of the updated workflow, add the new required check and confirm it is enforced on a pull request. Only then remove the superseded individual required checks. Repository settings are owner-managed. The automatic release workflow requires successful push-triggered CI for the exact main commit and publishes only validated version increases. See [Releasing](RELEASING.md) for version preparation and the current publication procedure.

## Rust coverage diagnostics

Coverage is optional and runs separately from `CI Gate`, `just verify` and release acceptance. On Windows x64, install the prerequisites explicitly from the repository root:

```powershell
pwsh -NoProfile -File scripts/coverage-rust.ps1 --install
just coverage
```

The installer uses the exact Rust version from `rust-toolchain.toml`, its matching `llvm-tools-preview` component and `cargo-llvm-cov` 0.9.0 with locked dependencies. A normal coverage run checks the prerequisites without installing or upgrading anything.

The command runs the same non-ignored Rust tests as `just test`, targeting `x86_64-pc-windows-msvc` with the checked-in Cargo lockfile. It prints a coverage summary and writes an HTML report and `summary.txt` in a new `.scratch/coverage/<run-id>/` directory. Open the printed `html/index.html` path in a browser. Instrumented binaries and profile data stay under `target/llvm-cov-target/`; `just clean` removes them with ordinary build output. Remove old coverage reports from `.scratch/coverage/` when no longer needed. Neither location belongs in source exports or change packages.

Coverage shows which code was exercised, not whether its behavior was tested well. Use unexercised paths to identify meaningful missing regressions. Ignored real-media tests and platform-inapplicable code are outside the measured run. There is no percentage threshold.

The separate `Coverage diagnostics` workflow runs on relevant pull request changes, weekly on Monday at 07:00 UTC and by manual dispatch. It uses the same script and uploads available reports for seven days. Instrumentation or test failures fail that workflow, not the required CI gate.

## Workspace commands

```powershell
just zip repo
just zip workspace
just review --files
just package my-task --tracked-only
just apply --zip 'C:/Downloads/apollo-task-changed-files.zip' --dry-run
just restore --backup 'C:/Backups/apollo import' --dry-run
just clean --dry-run
```

`just clean` removes only the project's `target/` directory. Tracked files require explicit `--force`; invalid roots, links and protected local content are always refused. Offline bundles, local plans and staged release artifacts outside `target/` are not cleanup targets.

See [repository scripts](../scripts/README.md) for all transfer and cleanup options. Offline preparation and maintenance are owner-managed, outside Just.

For module ownership, media policy and native assets, see [Architecture](ARCHITECTURE.md).

For release packaging, use `just build-release`, `just stage-release` and `just verify-release` as described in [Releasing](RELEASING.md). These commands are separate from ordinary verification. `just stage-media-tools` and `just build-installer` support direct diagnostic staging without rebuilding Apollo.

## Local workspace roots

The repository reserves these local roots:

```text
.plans/      local implementation plans
.project/    local project records and reviews
.scratch/    disposable working data
.agent-work/ owner-managed offline tooling and local files
```

They are intentionally ignored by Git and must never become build prerequisites. A clean clone must build and test without them.

Tracked development tooling belongs under `scripts/`. Offline bundle preparation is owner-managed; `.agent-work/`, generated archives and retired local bundle directories are excluded from workspace snapshots. Legacy planning paths remain passively protected by Git ignore rules.

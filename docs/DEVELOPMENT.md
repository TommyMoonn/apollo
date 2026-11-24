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

FFmpeg and FFprobe run as separate processes. The Windows release packages include the pinned pair; `cargo build` does not download or copy media tools. Apollo discovers and validates them through one shared application owner rather than feature-specific checks.

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

See [Testing](TESTING.md) for focused tests, `just verify`, optional tooling checks and native Windows acceptance.

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

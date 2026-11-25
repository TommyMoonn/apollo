# Apollo

Apollo is a local-first desktop media utility for Windows x64, built with Rust, egui and eframe. It runs FFmpeg and FFprobe as external processes for audio mixing, video upscaling, remuxing, audio extraction and frame capture. Users can run jobs individually or add them to a queue, with batch setup for multiple sources.

## Engineering philosophy

Build the simplest solution that fully solves the problem. Challenge assumptions and remove unnecessary complexity.

Follow YAGNI (You Aren't Gonna Need It). Before adding code, ask whether the behavior is needed and whether an existing pattern, standard library, platform capability or installed dependency can handle it. Avoid new dependencies for trivial helpers and abstractions for hypothetical needs.

Prefer deletion and reuse over unnecessary wrappers, single-use traits, configuration for values that never vary and speculative scaffolding. Favor designs that make invalid states unrepresentable and eliminate failure modes. Keep changes focused without overlooking callers or edge cases.

Never sacrifice correctness, input validation, data integrity, error handling, security, accessibility or requested behavior for simplicity.

## Protect user data

Treat source media and existing destination files as user-owned content. Preserve source files, reject input/output collisions and account for destinations reserved by queued jobs.

Use `MediaOutput` to validate destinations, manage temporary output and publish files without overwriting existing content. Publish results only after the process succeeds. If a job fails or is cancelled, remove only its own temporary output and keep existing files intact.

Treat media paths, FFprobe metadata and process output as untrusted input. Validate them in the relevant media or operation module, even when the UI has already checked them. Preserve settings compatibility in `src/settings.rs` and handle changes to persisted formats explicitly.

Media processing stays on the user's machine. Do not add accounts, uploads or network-dependent processing unless the task asks for them.

Apollo targets Windows x64. Check the native implementation and CI before extending platform support or making cross-platform claims.

## Before changing code

Read the relevant implementation, tests and documentation before editing. Trace affected callers, state transitions and execution contracts. Fix bugs at their source rather than patching individual callers.

Use the Rust toolchain selected by `rust-toolchain.toml` and run Cargo with `--locked` for routine builds and checks. Update `Cargo.lock` through Cargo when dependencies change. Repository tooling uses PowerShell 7 and Just. See [the development guide](docs/DEVELOPMENT.md) for setup and commands.

## Where code lives

- `src/main.rs` starts the native app. `src/app/` owns renderer-independent application state and route transitions.
- `src/ui/` contains operation screens, queue and batch views, shared controls, layout, typography and themes. `src/ui/legacy/` hosts the egui application adapter, its editor state and feature coordination.
- `src/media/` handles media intake, probing, tool discovery, stream models, child processes, output publication and progress parsing.
- `src/queue/` handles queued requests, destination reservations, execution order and job state.
- `src/audio_mix/`, `src/upscale/`, `src/remux/`, `src/audio_extract/` and `src/frame_capture/` contain editable drafts, operation requests, validation, command construction and execution. Frame Capture also handles frame indexing, previews and clipboard extraction.
- `src/settings.rs` persists remembered source and output directories. `src/file_dialog.rs` handles native file dialogs. `src/progress_repaint.rs` coordinates repaints in response to progress updates.
- `tests/` contains Rust integration tests, FFprobe fixtures, the optional tooling smoke check and Windows release acceptance support. Rust unit tests live alongside the implementation.
- `scripts/` contains repository and release tooling. `distribution/` contains media-tool version pins and acceptance records. `installer/` contains Windows installer sources. `docs/` contains engineering guides.

## Keep ownership clear

`Application` owns the route, media-tool discovery, source intake, queue and immediate executors. `ApolloApp` owns one core and coordinates the egui shell and background work. Keep route transitions in the core and renderer-dependent presentation and worker state in the adapter. Each operation owns its typed draft and request construction in `draft.rs`. Normal forms and independent editors reuse these drafts; the adapter retains presentation, lifecycle feedback and action routing. Keep execution validation in the operation validators and executors.

`SourceSession` in `src/app/source.rs` owns source acceptance, worker settlement and session retirement. `MediaIntake` remains responsible for source identity, retained probe results and pending requests. Keep accepted source presentation and drafts intact until replacement succeeds. `BatchInspectionOwner` manages batch probing and limits it to two concurrent probes. Preserve request identity checks so results from an earlier request cannot replace the current source or batch state.

`MediaQueue` owns queued jobs, `JobId` values, destination reservations, source revalidation and sequential execution. Only Waiting jobs can be edited or reordered, subject to the queue run state. Keep batch insertion atomic. Preserve the difference between cancelling the current job, stopping after it and cancelling the queue. Batch setup remains temporary until the queue accepts the requests.

`MediaExecution` manages processing and finalizes staged output for file-producing operations. Operation executors prepare validated requests and structured commands. `MediaProcess` manages child processes and diagnostics. Frame preview and clipboard extraction have their own owners in `src/frame_capture/`. Extend these execution paths instead of creating competing workers or output publishers.

`MediaToolsDiscovery` in `src/app/discovery.rs` owns availability, the active discovery attempt and capability settlement. `src/media/tools.rs` resolves and validates FFmpeg and FFprobe and caches encoder capabilities by executable identity. Preserve the adjacent-pair-first discovery policy and explicit NVENC failure behavior described in the [architecture guide](docs/ARCHITECTURE.md).

Give each piece of state one owner. Keep temporary presentation state in its UI or editor, shared application state in the application and processing policy in the relevant operation or media module.

## Write clear Rust and PowerShell

Return `Result` for fallible Rust operations and preserve useful error context. Use enums when states have different valid shapes. Avoid `unwrap()` and `expect()` in fallible production paths. Keep necessary `unsafe` code and lint suppressions narrow, and explain the invariant or constraint that justifies them.

Pass FFmpeg and FFprobe arguments separately to the process. Map streams using probed indexes and preserve the operation's stream-copy and encoding policy. Use the process exit status to determine success; progress reports alone cannot establish success.

Reuse `scripts/Cli.Common.ps1` for argument parsing and CLI output, and `scripts/Repository.Common.ps1` for shared Git and filesystem operations. Resolve paths explicitly and use literal file operations for user-provided paths. Follow the CLI conventions in [the script guide](scripts/README.md).

Prefer early returns and readable control flow. Write comments for invariants and external constraints that the code cannot express. Do not repeat what the code or its names already say.

## Build a consistent, accessible interface

Follow the current interface and the typography and layout guidance in [the architecture guide](docs/ARCHITECTURE.md). Reuse the shared controls, layout, theme and typography definitions in `src/ui/widgets.rs`, `src/ui/layout.rs`, `src/ui/theme.rs` and `src/ui/fonts.rs`.

Handle applicable loading, empty, error, success and cancellation states. Keep dimensions stable when feedback changes. Preserve keyboard access, visible focus, readable labels and appropriate hit areas. Follow the shared motion policy and keep animations restrained.

Check affected source, queue and batch views at supported window sizes when changing shared layout or interaction behavior.

## Keep the app responsive

Keep media discovery, probing, preview decoding and processing off the UI thread. Reuse existing background workers, bound concurrent work and preserve cancellation and stale-result checks when sources, requests or active jobs change.

Do not probe media, access output files or launch processes just because a view repaints. Reuse retained inspection and capability results while they remain valid. Pass completion callbacks to workers and settle their results in the owning application state. Use `MediaProcessWake` and `MediaProgressRepaintOwner` for process notifications so progress and terminal results appear promptly without continuous repainting.

Guard actions that mutate files or start background work against repeated activation. Cancellation must stop only the process owned by the operation and leave its state consistent.

## Test behavior that can break

For bug fixes, reproduce failures in a focused test when practical. Reuse existing cases and fixtures before adding tests. Test at the lowest layer that proves the behavior. Avoid duplicate assertions, tests of implementation details and exact error wording unless the wording is part of the contract. Use explicit events or conditions with bounded waits instead of fixed sleeps. For timed behavior, supply controlled timestamps rather than checking elapsed wall-clock time. Remove brittle or redundant cases during each test review; replace useful coverage before retiring it. UI tests should protect interaction, readable and reachable controls, non-overlap and stable layout, not exact paint shapes, incidental pixel values or copied design constants.

Use temporary directories, disposable media and controlled process fixtures for tests. Cover output collisions, failed or cancelled processing, cleanup, stale results and queue state transitions where relevant.

Prefer focused Cargo tests, such as `cargo test --locked --test remux` or `cargo test --locked --lib <filter>`. Run `just fmt-check`, `just lint` and `just check` as appropriate to the change. Run `just verify` before release preparation or when a full verification pass is requested. The `CI Gate` check requires formatting, strict Clippy, Rust tests, an application build and release metadata validation on Windows for every PR and main push.

`just test-scripts` is an optional local smoke check for repository tooling. Keep it small and independent of application verification, CI and release packaging. Release metadata validation, artifact integrity checks and installed/portable acceptance remain part of the release process.

Follow [the testing guide](docs/TESTING.md) for tests with real FFmpeg, NVENC and native Windows behavior. A Linux build, mocked process or encoder inventory cannot establish Windows, GPU or rendered-interface behavior. Report any checks you could not run.

## Keep documentation useful

Keep setup, architecture and testing guidance in their dedicated guides:

- [Development guide](docs/DEVELOPMENT.md) for setup and local commands.
- [Architecture](docs/ARCHITECTURE.md) for module ownership, media policy and native resources.
- [Testing](docs/TESTING.md) for verification and Windows artifact acceptance.
- [Repository scripts](scripts/README.md) for export, review, transfer, recovery and cleanup.
- [Media-tool acceptance](distribution/media-tools-acceptance.md) for the pinned distribution and compatibility evidence.

Update user documentation when a supported workflow changes. Document architectural decisions and constraints that are hard to recover from the source. Do not document every method or component. Keep technical version history in `CHANGELOG.md` and the canonical GitHub release body in `release-notes/vX.Y.Z.md`. Follow [the releasing guide](docs/RELEASING.md) for version changes and candidate validation. A version increase merged to main can publish automatically after CI; prepare and review release metadata before merging.

## Planning and work artifacts

Keep plans, research, reviews, handoffs and evidence in local, untracked `.plans/`. Use `.project/` for local project reference material and `.scratch/` for disposable experiments. `.agent-work/` holds owner-managed offline tooling and local files. Leave skills and offline preparation tooling unchanged unless the task explicitly asks to update them.

Use `just zip repo` for committed source and `just zip workspace` for current files with eligible local context. Review local records before sharing workspace exports. Use `just package` for changed-file handoffs. Keep local plans, project records, owner-managed skills, offline files and generated output out of those packages. None of these local directories are build prerequisites.

## Git and handoff

- Review the working tree before editing. Preserve unrelated changes and never discard user work without permission.
- Follow the requested Git workflow. Commit, push or create a PR only when directly requested or authorized by an explicitly invoked workflow.
- Use concise Conventional Commit subjects for commits and PR titles. Scopes are optional, e.g., `fix(queue): preserve waiting job order`. PR descriptions should explain the problem, solution and validation.
- Before handoff, review the final diff, run relevant checks and report results and anything unverified. Include only the requested changed files, identify deletions and keep local planning records separate.

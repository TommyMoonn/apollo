# Apollo Architecture

> [!IMPORTANT]
> All of the documentation here are written by LLMs.

Apollo is a native Windows x64 media utility written in Rust. eframe hosts an egui interface, and FFmpeg and FFprobe run as separate child processes. Media stays on the user's machine. The application has no web frontend, server or account dependency.

This guide describes the egui implementation in the current checkout.

## Module boundaries

| Area | Responsibility |
| --- | --- |
| `src/main.rs` | Native entry point and window configuration. |
| `src/app/` | `Application`: renderer-independent route state, dialog requests, preferences, media-tool discovery, source intake, interactive frame coordination, queue and immediate executors. |
| `src/ui/legacy/` | `ApolloApp`: egui composition, ordered background updates, renderer-dependent editors and existing feature actions. |
| `src/ui/` | Operation views, shared widgets, queue and batch views, layout, themes and typography. |
| `src/media/` | Source inspection, media models, tool discovery, process lifecycle, progress and output publication. |
| `src/queue/` | Queued requests, job identity and execution order. |
| `src/audio_mix/`, `src/upscale/`, `src/remux/`, `src/audio_extract/`, `src/frame_capture/` | Operation requests, validation, command construction and execution. |
| `src/settings.rs` | Typed remembered-directory preferences, JSON conversion and fallback policy. |
| `src/file_dialog.rs`, `src/native.rs` | Native dialog service, parent window handles and shell folder actions. |
| `src/progress_repaint.rs` | Callback-based progress notifications. |

The library exports the application core and UI modules through `src/lib.rs`. The native binary configures the window and constructs the egui adapter, which owns exactly one core. The core does not import UI or native adapters. Keep operation policy in its domain module and use the shared media and queue owners. Views display state and collect input; repainting must not start probes or processing.

## Composition and navigation

`src/app/navigation.rs` owns `Operation`, routes and return origins for queue and batch flows. Route transitions do not require an egui context. The adapter commits these transitions after the existing source, editor and queue actions succeed; it retains renderer-dependent editor presentation and worker coordination. Queue return and batch submission preserve the original source operation. Queue history changes remain with `MediaQueue`.

`src/ui/legacy/mod.rs` constructs the application, installs the existing font and theme definitions and coordinates background updates and shutdown. `render.rs` dispatches the active route and draws the shell. `source.rs` handles source actions and presentation settlement. The four ordinary operation adapters (`audio_mix.rs`, `upscale.rs`, `remux.rs` and `audio_extract.rs`) own their ready surfaces, immediate actions and lifecycle feedback. `dialogs.rs` routes native completions; `frame_capture.rs` routes frame actions and uploads textures; `queue.rs` translates queue effects into native actions and presentation updates; `batch.rs` presents batch setup and fixed-source editors and routes native actions; `workflows.rs` retains shared route and lifecycle helpers; `state.rs` retains adapter-specific presentation.

Each root update first polls media-tool discovery and queue events. While base tools are checking or unavailable, it renders that status and returns. Once ready, the update polls dialogs, source inspection, batch inspection, frame preview, frame index, clipboard and thumbnails in that order before handling drops and rendering the active route. Shutdown retires interactive frame work and batch inspection, then cancels the queue. Dropping an active file execution stops and joins its child worker before discarding staged output. Interactive preview retirement remains nonblocking.

## Source intake and execution

`src/app/source.rs` owns the accepted source session, its inspection worker and intake errors. `MediaIntake` remains the authority for source identity and probe acceptance. An unsuccessful replacement leaves the accepted session intact; successful replacement or restoration from a queued snapshot creates a new session identifier. Closing a source retires its pending inspection and accepted session without touching queued jobs. Source and independent editor identifiers share one allocation sequence to keep stale worker and dialog results from targeting a different editor.

The legacy source adapter builds one immutable `SourcePresentation` after acceptance or queued-source restoration. All normal operation drafts share the accepted source identifier and retain their settings when switching operations. Independent queue and batch editors keep their own snapshots. Presentation and frame support are retired together when the accepted source changes or closes.

1. `MediaIntake` identifies the source using its canonical path, size and modification time. It reuses a successful inspection only when the identity is reliably unchanged, and retires results from superseded requests.
2. `src/media/probe.rs` invokes FFprobe, parses JSON and normalizes streams into the models in `src/media/model.rs`. `src/media/container.rs` owns shared MP4 and Matroska stream-copy compatibility.
3. Operation drafts validate typed requests against inspected media without filesystem access. Queue admission checks source identity and the output parent before retaining request and media snapshots; batch admission performs the same checks for each item before inserting any job. At launch, the queue rechecks the retained source identity. Both immediate and queued starts then call the same operation-specific preparation functions, which validate requests again rather than trusting draft readiness. Audio Mix reuses the validated channel layout, and Upscale reuses its calculated dimensions, resolved encoder and pixel format when constructing the command. Command builders use the staged destination directly, and `PreparedMediaOperation` configures FFmpeg progress on stdout.
4. `MediaExecution` owns each active file operation and settles its output once. `MediaProcess` owns the child, stream readers, cancellation, bounded stderr diagnostics and progress channels. Cancellation wakes the child polling loop; terminal state is published after the child exits and both readers finish. A reader failure cannot turn a successful exit into a completed operation. `src/media/progress.rs` parses progress records; an unknown duration remains indeterminate, and progress alone cannot establish success.
5. `MediaOutput` checks the destination, owns temporary output and publishes it only after successful processing. At execution start it selects an available numbered name for progress display. The operating system’s no-replace move remains authoritative at publication: if that name becomes occupied, publication tries the next numbered name. Admission does not reserve destinations. Failed or cancelled jobs clean up their own temporary output without overwriting the input or an existing destination. Publication failure is terminal; only cleanup may be retried during disposal.

The five immediate executor wrappers still own their individual presentation lifecycles. The queue owns a separate `MediaExecution` and advances sequentially. Both use the same preparation and output settlement; they do not share an active execution slot yet.

Pass process arguments separately instead of constructing shell commands. Map streams by their probed indexes. Preserve request identity, cancellation and completion routing when adding background work. Frame Capture also has dedicated frame-index, preview and clipboard workers under `src/frame_capture/`; clipboard extraction does not publish a file.

## Interactive Frame Capture

`src/app/frame_capture.rs` owns renderer-independent editor state, debounce and step decisions, thumbnail request state and the worker lifecycle. Preview, index and clipboard work is tied to the originating editor identifier and source identity; preview and index generations also identify the exact request. Normal, queue and item-override editors retain separate state. Exact frame positions are independent of the formatted timestamp and decoded pixels. The legacy frame adapter routes active-editor actions, supplies notification callbacks and uploads egui textures only for accepted generations.

The existing `FramePreview` and `FrameNavigator` retain their bounded caches; no second decoding or neighborhood cache is used. Explicit position changes retire superseded work. Temporary navigation away suspends outstanding work while retaining accepted pixels and cached neighborhoods, and returning schedules any necessary work. Clipboard extraction remains full-resolution and independent of file output and queue jobs. A retired decode cannot claim clipboard publication; a native publication already claimed may finish. Pixel conversion occurs before opening or clearing the clipboard, and shutdown does not wait on a native clipboard call.

## Native dialogs and preferences

`FileDialogOwner` owns the single pending request, Windows COM dialog service, completion callback and cancellation result. `src/native.rs` supplies the parent window handle (also used by clipboard operations) and native folder opening. `src/app/dialogs.rs` prepares input directories and builds output requests from typed drafts. The legacy dialog adapter routes results to the initiating source, batch or editor. Input requests capture the navigation revision and accepted source identifier; a retired context cannot receive a result even after returning to the same route. Output results target one operation and session, and only accepted selections update the remembered directory. Cancellation, errors and stale results do not change preferences.

`Application` owns `AppSettings`. `src/settings.rs` contains the existing JSON fields and directory fallback policy without eframe dependencies. `src/ui/legacy/preferences.rs` alone reads and writes the `apollo.settings.v1` eframe storage key at the existing persistence location. Missing or malformed values load defaults, unavailable directories fall back without discarding the saved value, and unchanged preferences are not rewritten. Source selections remember the selected directory even if subsequent media inspection fails.

## Operation contracts

| Operation | Contract |
| --- | --- |
| Audio Mix | Mix selected embedded audio tracks into one AAC stream while copying the video. |
| Upscale | Fit video within the selected bounds, preserving aspect ratio, with H.264 or H.265 encoding. |
| Remux | Copy compatible streams into MP4 or MKV without re-encoding. |
| Audio Extract | Map one selected audio stream to original MKA, AAC or FLAC output. |
| Frame Capture | Extract one video frame as PNG or JPEG; preview and clipboard paths have separate lifecycles. |

Each operation owns its editable configuration in `draft.rs`, alongside its request model, validator, command builder and executor. Normal forms and queue and batch editors use the same typed request construction. `src/media/output_name.rs` owns shared filename stem and directory editing; the selected operation supplies the extension. Invalid editable text stays in the draft until corrected. Audio Mix caches pure readiness validation and invalidates it when draft or relevant source metadata changes. Upscale uses the current discovery capability state: explicit CPU is available while NVENC is pending, whereas Auto and explicit NVENC wait for settlement. Immediate executors and queue execution repeat their applicable validation before starting work. Extend these paths instead of creating alternate processing pipelines in UI code.

## Media-tool discovery

`src/app/discovery.rs` owns the discovery lifecycle, including checking, ready and unavailable states, the active attempt and capability settlement. `src/media/tools.rs` resolves FFmpeg and FFprobe and performs bounded version validation. Apollo first validates the complete pair beside its executable. If neither file exists there, it resolves both on PATH. A partial or invalid adjacent pair is reported as an error without falling back to PATH. Retry uses the same policy.

One background worker validates the base tools, publishes their paths, then checks optional NVENC capabilities. Retry is single-flight; the owner rejects stale attempts and capabilities for a different tool pair. A capability failure after base discovery leaves the base tools usable. Encoder capability results are cached by resolved FFmpeg path, file size and modification time. This identifies the runtime cache entry; release staging separately verifies the pinned archive checksum. The distribution contract lives in [media-tools.json](../distribution/media-tools.json), with measured compatibility in [the acceptance record](../distribution/media-tools-acceptance.md).

## Worker notifications

Workers publish results to their existing channels before invoking a completion callback. When a panic is reported through disconnection, the worker drops its result sender before notifying. The callback carries no application state; the owner consumes and validates the result on the application side. The egui adapter supplies a repaint callback for discovery, inspection, preview decoding, thumbnail loading, native dialogs and clipboard completion. `MediaProcessWake` carries process progress and terminal events. `MediaProgressRepaintOwner` coalesces progress notifications at 34 ms, rejects stale operation generations and wakes immediately on terminal events. Idle workers do not schedule periodic repaints. Frame Capture returns debounce deadlines to the adapter, which schedules delayed repaints.

## Queue and batch ownership

`src/app/queue.rs` owns application queue commands, edit admission and settlement, and the active-job notification channel. It passes typed requests and commands to the single scheduler and supplies renderer-neutral wake callbacks. `ProcessExecutionArbitration` uses immediate executor, clipboard and queue state directly. `src/app/editor.rs` owns fixed-source draft state and request construction shared by queue and batch editors; textures and source presentation remain in the adapter.

`MediaQueue` owns queued request snapshots, monotonic `JobId` values, source revalidation, sequential advancement, cancellation, terminal history, and atomic batch insertion. Single and batch admission require the source identity retained at inspection, validate it against the request and current filesystem, and apply the operation validator and output preflight without staging files. Batch admission checks all candidates and the full ID range before appending jobs; queue edits revalidate against the retained identity. Launch still rechecks source and destination. Queued jobs use `MediaExecution` for processing and staged-output publication. Queue progress and completion are routed by the active `JobId`; repainting a Queue or Batch surface must not inspect media or touch the filesystem. Tool readiness gates new launches, not settlement. If readiness is lost during a run, the active job settles without cancellation and automatic advancement stops, preserving waiting jobs for an explicit restart.

Only Waiting jobs are editable or reorderable, and editing requires a stopped queue with no active execution. Queue-row Edit and the loaded-source queued-item selector enter the same queue editor and hydrate the retained `MediaFile` and immutable request without another FFprobe invocation. The source is fixed, while the operation and settings may change. Saving replaces the Waiting request atomically without changing its `JobId` or queue position. Entry origin owns the return route.

`Add to queue` from a normal source workspace keeps the source and operation active, increments the pending count, and shows operation-local success or failure feedback. Pending counts include Waiting and Running jobs only. Multiple jobs may target the same requested filename. Completed rows display the actual published path.

`src/app/batch.rs` owns temporary batch setup, inspection settlement, shared defaults, overrides, inclusion and admission. Stable `BatchItemId` values associate row actions, inspection results and admission errors with their originating item; the worker index is resolved through the inspection session’s item IDs. It applies one shared operation and source-independent defaults, then projects and validates a request independently for each inspected source. Per-file overrides survive later shared-default changes. `BatchInspectionOwner` supplies at most two concurrent probes per active inspection session. Retirement cancels remaining work and discards late results; an already-running probe can finish. Appending sources uses the same inspection engine and preserves the current operation, defaults, inclusion choices, and overrides. Repeated sources are allowed as independent items. No queued job exists until `MediaQueue` accepts the complete included batch atomically.

Queue controls:

- `Cancel job` requests cancellation of the active job and continues advancement only while the queue is Running. The active job remains owned and displays Cancelling until process and output settlement finishes.
- `Stop after current` lets the active job finish, stops advancement, and preserves Waiting jobs.
- `Cancel queue` requires confirmation, stops advancement, marks all Waiting jobs Cancelled and requests active cancellation. An already claimed terminal result is not replaced by cancellation.
- `Clear finished` removes every terminal job, including failures.
- Queue history is application-scoped for the current session. Back, Add files, source replacement and tool changes retain all jobs. Clear finished removes terminal records only, never output files. Opening Queue from batch setup suspends the view without retiring the batch; Back restores it. New file intake is disabled while that batch is suspended to avoid replacing the retained draft.

## Upscale sizing and encoding

`src/upscale/sizing.rs` calculates even output dimensions that preserve the source aspect ratio within the selected bounds. Presets account for orientation; custom width and height must be positive and even and are never swapped. Sizing rejects bounds that cannot enlarge the source without exceeding a boundary or reducing a source dimension through quantization. Execution uses the calculated dimensions.

Scaling uses `scale=<width>:<height>:flags=lanczos`. Encoder selection does not enable hardware decoding or CUDA/NPP scaling.

| Backend | H.264 | H.265 | Quality policy |
| --- | --- | --- | --- |
| CPU | `libx264`, CRF 18 default | `libx265`, CRF 20 default | CRF 0..51, `slow` preset |
| NVIDIA NVENC | `h264_nvenc`, CQ 16 default | `hevc_nvenc`, CQ 16 default | CQ 1..51, `p5`, VBR, explicit CQ |

Lower quality numbers favor quality over file size. CRF and CQ are not interchangeable scales. State retains values independently for each codec/backend, alongside the last custom boundary and manual output name.

The shared media-tool owner checks compiled NVENC availability and runs bounded, hidden, minimal real encode probes off the egui thread. H.264 and H.265 results are independent and cached for the resolved FFmpeg identity. Base tool discovery can finish before these probes: normal workspace use and CPU Upscale remain available while Auto and explicit NVENC show checking state. Auto selects NVENC only after success; explicit unavailable NVENC retains diagnostics and never silently falls back. Do not probe on repaint or command construction.

The measured NVIDIA baseline and API version are recorded in [the media-tool acceptance record](../distribution/media-tools-acceptance.md). Encoder inventory alone cannot prove runtime driver compatibility. If initialization fails, Auto resolves to CPU; explicit NVIDIA selection retains the real probe error. An error after an encode has started remains a job failure, not a silent CPU rerun.

### Pixel format and color limits

The probe retains pixel format, derivable bit depth, range, matrix, transfer, and primaries. Shared Upscale validation resolves the output format for both ready feedback and execution:

- 8-bit `yuv420p` is supported on CPU and NVENC. `nv12` uses `yuv420p` on CPU or `nv12` on NVENC; `yuvj420p` is CPU-only.
- Supported 10-bit 4:2:0 sources require H.265, using `yuv420p10le` on CPU or `p010le` on NVENC. PQ/HLG signaling requires known compatible 10-bit input.
- Unsupported or contradictory known format information is rejected. When both pixel identity and depth are absent, strict FFmpeg format selection preserves the filter output rather than guessing an 8-bit format.
- Commands use strict `-pix_fmt` selection and map only available color values. External encoder/hardware limitations can still fail execution instead of silently choosing a different format.
- Mastering-display and content-light side data are not explicitly preserved or verified. Format checks alone cannot establish full HDR preservation or visual fidelity.

## Native interface and resources

`src/ui/layout.rs`, `src/ui/widgets.rs`, `src/ui/theme.rs` and `src/ui/fonts.rs` provide the shared layout, controls, appearance and typography. IBM Plex Sans is the primary interface font; Fira Code is used for the Apollo wordmark and compact technical metadata. Both are embedded at compile time from `assets/fonts/`. Missing assets fail the build. Preserve the font licenses and asset identities documented in [the font notes](../assets/fonts/README.md).

Keep source inspection, preview decoding and media processing off the UI thread. `MediaProgressRepaintOwner` coordinates progress notifications; retained state should drive the interface between updates.

`src/ui/icon.rs` embeds `assets/apollo-logo.png` for the native window. On Windows, `build.rs` compiles `assets/apollo.rc` using Cargo's package version. The resource script embeds `assets/apollo.ico` and the executable's Windows version information. The NSIS installer also consumes the ICO. Documentation artwork in `assets/docs/` is not used by the application build.

## Distribution boundary

`distribution/` owns the media-tool pin and compatibility evidence, `scripts/` owns staging and release tooling, and `installer/windows/` owns the NSIS installer. The installed and portable distributions include the pinned media tools and required license material. Cargo builds do not download or stage those tools.

See [Development](DEVELOPMENT.md) for setup and [Testing](TESTING.md) for verification and artifact acceptance.

# Apollo UI design

## Reference and ownership

Apollo is a compact, source-first media utility. Preserve the existing v0.7.0 workflow, operation order, defaults and technical options. The historical `.project/retired/design/` records and `assets/docs/apollo-preview.png` are reference material, not pixel-exact targets. [Sonora at c1611b05](https://github.com/sonorahq/sonora/tree/c1611b05f79f6367903bd9d6a419190c9912b477) is a visual-quality reference for typography, geometry and control consistency, not a source for Apollo's palette, layout or GPUI APIs. The queue uses a dedicated GPUI workspace; scheduler controls and job actions land in successive U02 phases. [Official GPUI at Zed v1.29.3 (d01b134)](https://github.com/zed-industries/zed/tree/d01b134cddf5e4f71964deb2d6f81cdb38fcf1e7/crates/gpui) is the API reference.

`src/ui/design.rs` owns semantic colors, typography, spacing and geometry; `src/ui/components.rs` owns reusable GPUI elements; `src/ui/shell.rs` owns the shared header, body and footer layout; `src/ui/root.rs` owns the window and the single application entity. Feature views compose controls from supplied state and callbacks. They never start media work on render or duplicate domain state. Retained egui controls and fonts serve the old views and tests until those views are ported.

## Color and typography

Dark is the default. Light roles are defined but not selectable yet. U07 adds appearance settings, zoom and System theme updates.

| Role | Dark | Light |
| --- | --- | --- |
| Background | `#000000` | `#FFFFFF` |
| Foreground, progress and focus | `#FFFFFF` | `#000000` |
| Secondary | White, 168/255 alpha | Black, 168/255 alpha |
| Faint | White, 122/255 alpha | Black, 122/255 alpha |
| Separator | White, 46/255 alpha | Black, 46/255 alpha |
| Interactive border | White, 77/255 alpha | Black, 77/255 alpha |
| Hover / pressed / selected fill | White, 14/28/28 alpha | Black, 14/28/28 alpha |
| Disabled | White, 87/255 alpha | Black, 87/255 alpha |
| Primary fill / text | White / black | Black / white |
| Primary hover / pressed | White, 224/194 alpha | Black, 224/194 alpha |
| Destructive interaction border | `#7E3030` | `#7E3030` |

Alpha values are out of 255 and composite once over the local surface. Use foreground or secondary for information needed to act; faint is optional metadata only. Do not dim a whole tree or communicate status by color alone. Selection uses subtle fill and an outline, not a colored strip. Destructive actions retain readable text, using red only on hover/press borders.

### Typography roles

Resolve typography through `TextRole` in `src/ui/design.rs`. Each role specifies a family, weight, size and line height. Use `TextRole::apply` for rendering and `TextRole::resolve` for measurement. Colors remain separate palette roles.

| Role | Family | Size / line height (logical px) | Weight | Usage |
| --- | --- | --- | --- | --- |
| Wordmark | Fira Code | 26 / 32 | 400 Regular | Lowercase `apollo` on the landing page |
| Page title | Geist | 20 / 26 | 600 Semibold | Queue, Settings and editor titles |
| Section heading | Geist | 14 / 20 | 600 Semibold | Source identity, group headings and confirmation titles |
| Body | Geist | 14 / 20 | 400 Regular | Ordinary text, values and input contents |
| Control | Geist | 14 / 20 | 500 Medium | Buttons, selectors, field labels and action links |
| Status | Geist | 14 / 20 | 500 Medium | Readiness, pending work and job state |
| Secondary | Geist | 12 / 16 | 400 Regular | Metadata, units, paths and supporting instructions |

Control and Status use the font's OpenType tabular figures (`tnum`) so numeric counts and progress labels remain stable as digits change. Both measurement and rendering use this same feature setting. These are the defaults at 100% UI zoom. U07 owns UI font choice, text size and zoom; the fixed brand family and size remain independent of UI font selection and text size. The OS window title remains Apollo.

Both font assets are embedded and registered once at startup. Geist's variable weight axis supports the requested 400, 500 and 600 weights; Fira Code contains its Regular face. Missing glyphs use Segoe UI, Segoe UI Symbol and DirectWrite system fallback. The pinned GPUI text style API has no supported letter-spacing property, so the wordmark uses the font's native tracking. See [font assets](../assets/fonts/README.md) for asset metadata and provenance.

The opaque window uses GPUI's platform-default text rendering, allowing ClearType when enabled in Windows. Grayscale is the fallback. Native face selection, rendering and DPI acceptance must be checked on Windows; requesting a weight does not prove DirectWrite selected that face.

## Geometry and interaction

| Role | Logical px |
| --- | --- |
| Content inset | 28 |
| Tight / compact / related / focal / section / spacious spacing | 4 / 7 / 11 / 18 / 24 / 40 |
| Action bar inset / gap | 10 / 8 |
| Primary / compact control minimum height | 31 / 30 |
| Selectable row minimum height | 34 |
| Control / container radius | 4 / 4 |
| Rule / progress radius | 1 / 0 |
| Focus width / preferred offset | 2 / 2 |

Use GPUI logical pixels and native scaling. The reference drawable area is 730 × 489; minimum is 400 × 260. Layout uses flex, wrapping and intrinsic sizing, with no horizontal page scrolling. The source header, selector and footer stay fixed when measured chrome leaves usable body space. The body scrolls independently in that mode. At short heights, the entire page scrolls instead, with no nested scroll region. The source identity and actions reflow as groups; long names and paths truncate with full text available in tooltips. Never squeeze labels, reduce zoom automatically or render content beneath a footer.

Use one border per edge. The shared button uses a 1 px border, replaced by a 2 px focus border with compensating padding so focus does not resize the control. Primary focus uses a contrasting inset border. Destructive focus also underlines the label so the red hover border cannot hide focus. The preferred external focus offset is not yet implemented in GPUI; native inspection must confirm readability. Tab and Shift+Tab move focus; GPUI's focused click handling activates on Enter/Space. Disabled and busy actions have no click handler or tab stop. Buttons expose a GPUI accessibility role and action name; busy controls expose an in-progress description without replacing their accessible name. The pinned GPUI API does not expose a dedicated busy/disabled flag for these elements. Native UI Automation announcements require further verification. Use explicit tooltips for any later icon-only action, not as the sole reason or instruction.

Motion is restrained. The reference surface/detail/hover timings are 120/110/85 ms. Pending work uses the shared 16 px spinner with a 900 ms cycle; the SVG is embedded through the GPUI asset source. At startup Apollo reads the Windows client-area animation preference and passes it to GPUI’s reduced-motion setting. GPUI renders repeating animations statically when reduced motion is enabled. Live preference changes and application appearance controls remain in U07. No idle animation is scheduled.

## Shared components

| Component in `src/ui/components.rs` | Contract / current consumer |
| --- | --- |
| `Button` | Primary, standard, quiet and destructive tones; an explicit state that carries the activation callback only when enabled or selected. All states use the Control role. Alternate-label controls reserve the widest shaped label, rounded up as GPUI rounds intrinsic text, plus padding, border and any supplemental icon slot. Other buttons use GPUI intrinsic sizing with a minimum spinner footprint. Busy controls use the measured idle label width, keep their tone and height, and replace their label with a centered spinner. Heights accommodate the line height and padding. |
| `icon_action` | A supplemental glyph with a visible action label; no unlabeled icon-only controls yet. |
| `selector`, `selectable_row` | Wrap whole controls and use the button selection treatment. Workspace operation selectors use these controls. |
| `heading`, `page_title`, `separator`, `status`, `inline_message`, `empty_state` | Concise headings and necessary information, with readable inline status and optional Details group. The shell consumes separator, status, inline messages and empty state. |
| `spinner`, `centered_spinner`, `progress` | Shared inline and centered indeterminate indicators and a 30 px reserved progress region. Determinate progress requires a finite measured fraction; nonfinite or unknown values use the same spinner. Waiting and idle regions do not animate. Use stable IDs for actual pending owners. |
| `action_bar` | Wrap complete controls without shifting labels; shared bottom action area. |

Construct a `Button::new` with a stable element ID, explicit tone and label. Set `state` to `ActionState::Enabled(Box::new(callback))` or `Selected` only when the owner permits activation; pass the current palette and window to `render`. Supply all alternate labels when switching between them must not change control width. Views do not recreate the button border or hover styles. The shell displays Retry when discovery fails, actual queue activity and a reserved-width Open files control. Open files and Change file share native multi-selection and external file-drop admission. The landing screen centers the lowercase 26 px Fira Code wordmark at the top, outlined Open files and drop instruction in the body, and equal-width outlined Queue and Settings at the bottom. Measure the footer against both labels and `Queue (99)`, growing both buttons together for larger counts. `Queue` appears at zero; active and waiting jobs contribute to `Queue (N)`, terminal jobs do not. The root uses a reserved border for drag feedback without moving content. During replacement inspection the accepted filename and summary remain visible, and Close source retires pending results. Errors show actionable guidance, not raw probe output. Multiple files enter the retained batch setup with a count and Back action; submission remains unavailable until U06. The source workspace provides the five operation selectors in domain order. Queue and Settings open temporary routes with Back and retain the accepted source, operation or batch origin. If inspection succeeds while a temporary route is open, its return origin updates without dismissing the route. Operation forms, waiting and terminal job management controls, batch configuration and appearance settings remain unavailable until their respective migration phases. The Queue workspace is available independently of tool readiness. While a native picker is open, its initiating action cannot be repeated. Cancellation and failure restore focus, and an accepted selection starts the existing intake lifecycle. Preference failures provide inline feedback and a retry action when a write is pending. Close confirmation uses shared buttons and replaces shell actions while the decision is pending. Focus moves to Keep working, Escape dismisses and restores previous focus, and confirmed close waits for settlement before removal. No dummy actions or fake media jobs are provided to demonstrate controls.

## Loading contract

Use actual owner state, not elapsed time or a separate loading store. Show one centered indicator in the affected body for initial media discovery, initial single-file inspection, unresolved batch inspection and shutdown settlement. Startup reveals the intake action when base tools become ready, even if encoder discovery continues. Discovery failure replaces that body with actionable guidance and Retry; Queue and Settings remain independently available. An initial source probe uses a centered indicator without a cancellation control. During replacement only Change file becomes busy; the accepted source and operation selection remain visible. Native picker feedback stays in the Open files or Change file slot. When inspection settles, the real result, error or migration gap replaces the indicator. Never represent an unported feature as loading.

Outside the Queue route, the shell reserves a 30 px queue progress slot above its navigation actions even when idle. Active jobs use the same slot for an unknown or measured fraction, including cancellation until settlement. A started queue waiting for capability discovery has static waiting text; a stopped waiting queue has no indicator. The slot does not change the body or navigation geometry as progress changes. Inside Queue, the active job has its own reserved progress region instead of a duplicate shell indicator. Measured percentages do not round up to completion; the final timestamp shows Finalizing output until publication settles. Processed media time is shown only when reported, never as a wall-clock estimate. Per-file batch indicators belong to U06.

For later views, preserve usable content during localized work. Supply the shared component a stable owner-scoped ID, use the same typography and control measurement in idle and busy states, and reserve progress/control regions before a task starts. No duplicate loading row, arbitrary delay, fake progress, renderer polling or worker ownership change. The accessible progress label identifies the operation, and a known percentage exposes a numeric progress value. Unknown progress and busy actions expose an in-progress description; native announcements and focus must be checked on Windows.

## Copy, rows and feedback

Use explicit verbs and concise titles, no redundant subtitles, slogans or descriptions of obvious actions. Preserve actual counts, destinations, units, prerequisites, validation and consequences. Keep status labels short and provide full accessible reasons for failure; do not put raw logs or internal IDs into the primary UI. Long identity strings may ellipsize with full text accessible, but status and action labels remain readable in full.

Queue uses a fixed header with Back, Queue, Add files and a live processing/waiting/finished summary. The scrollable body omits empty sections and orders Processing, Waiting and Finished; an empty queue displays No jobs yet. Rows retain source, destination, operation and status, with full paths available through tooltips and accessible descriptions. Running and completed rows show the actual destination, not a stale requested filename. Add files uses native intake and is unavailable while a retained batch is suspended. Batch setup remains a migration gap until U06. The header reserves one primary slot for Start queue, Stop after current, Stop queue, Stopping after current and Cancelling. Active cancellation stays with the active row and retains its geometry through settlement. Cancel queue is a secondary destructive action with a confirmation; declining or pressing Escape preserves the queue. Failed rows show concise recovery guidance. Waiting-job and terminal management actions land in later U02 phases. Queue and batch headers and rows share a column layout. Align text left, numbers consistently and actions right. Vertically center the complete identity block, status and action groups. Keep status and inline Details on the same baseline and size the shared column for its widest supported content. Reflow whole groups at narrow widths or higher zoom, rather than clipping labels or breaking Details onto a new line. Never render empty disabled Details placeholders. Errors expose full diagnostics through a keyboard-accessible action, not hover alone.

For the GPUI batch port (U06), retain A02 behavior: `Edit shared settings` / `Apply settings`; per-file `Edit settings` / `Save changes` / `Cancel`; `Skip` / `Include` without losing configuration; `Custom settings` only for overrides; `Checking`, `Ready`, `Needs review`, `Incompatible`, `Skipped`, `Couldn't open`; `Add N jobs to queue` (singular for one). The right-aligned action pair uses a fixed slot for Edit settings and an equal-width Skip/Include slot based on the wider measured label. Keep a missing Edit settings slot empty, not interactive. Reflow the pair as a unit. Show actual counts once and avoid redundant blocked-count text. The latest retained batch behavior opens diagnostics from the clickable affected status (underlined on hover); retain that explicit interaction instead of adding a redundant Details action back to batch. Other diagnostic groups follow the inline Details contract. Full error reasons, atomic admission, inclusion and overrides remain owned by existing batch/queue state.

## Verification and limitations

Review actual reference and minimum windows, representative long labels, disabled/busy/selected/focused controls, stable dimensions and keyboard operation. Inspect Windows rendering at 100%, 125%, 150% and 200% DPI, including borders, fallback glyphs, weight selection and scroll reachability. Verify the packaged executable opens without installed Geist or network access. Check that animation runs only while needed and reduced motion uses static feedback. The Linux verification environment does not establish Windows DirectWrite, accessibility or DPI behavior. The embedded font metadata and local SHA-256 checksums are recorded in the font notes and notices. Native Windows face selection, focus, DPI, packaged offline rendering and remaining feature controls are outstanding.

# Apollo UI design

## Reference and ownership

Apollo is a compact, source-first media utility. Preserve the existing v0.7.0 workflow, operation order, defaults and technical options. The historical `.project/retired/design/` records and `assets/docs/apollo-preview.png` are reference material, not pixel-exact targets. [Sonora at c1611b05](https://github.com/sonorahq/sonora/tree/c1611b05f79f6367903bd9d6a419190c9912b477) is a visual-quality reference for typography, geometry and control consistency, not a source for Apollo's palette, layout or GPUI APIs. The queue is the dedicated redesign; its new presentation lands in U02. Official GPUI 0.2.2 is the API reference.

`src/ui/design.rs` owns semantic colors, spacing and geometry; `src/ui/components.rs` owns reusable GPUI elements; `src/ui/root.rs` owns the window and the single application entity. Feature views compose controls from supplied state and callbacks. They never start media work on render or duplicate domain state. Retained egui controls and fonts serve the old views and tests until those views are ported.

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

Geist Variable Sans is the GPUI typeface, pinned to the official `geist` 1.7.2 package. The source asset is embedded in the executable and registered once on startup. Missing glyphs use Segoe UI, Segoe UI Symbol and DirectWrite system fallback. See [font assets](../assets/fonts/README.md) for the outstanding local font acquisition. Body and controls are 14 logical px, secondary text 12 px, section headings 14 px semibold and intake/title 20 px semibold. Use regular and semibold weights rather than tracking or synthetic font transformations. Font registration and actual variable-weight rendering require Windows acceptance.

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

Use GPUI logical pixels and native scaling. The reference drawable area is 730 × 489; minimum is 400 × 260. Layout uses flex, wrapping and intrinsic sizing, with no horizontal page scrolling. A single scrolling body keeps content reachable. Source header, selector and action bar will remain fixed where space permits once the actual workspace is integrated; short-height fallback puts whole groups into the same scroll owner. Never squeeze labels, reduce zoom automatically or render content beneath a footer.

Use one border per edge. The shared button uses a 1 px border, replaced by a 2 px focus border with compensating padding so focus does not resize the control. Primary focus uses a contrasting inset border. Destructive focus also underlines the label so the red hover border cannot hide focus. The preferred external focus offset is not yet implemented in GPUI; native inspection must confirm readability. Tab and Shift+Tab move focus; GPUI's focused click handling activates on Enter/Space. Disabled and busy actions have no click handler or tab stop. Visible text identifies actions; native UI Automation and icon-only accessible names require further verification. Use explicit tooltips for any later icon-only action, not as the sole reason or instruction.

Motion is restrained. The reference surface/detail/hover timings are 120/110/85 ms; no idle animation is needed. Pending work can use a reserved-size spinner, with a static variant for reduced motion. Determinate progress is shown only when a real fraction is available. The spinner icon is an original geometric SVG in `assets/icons/spinner.svg`, embedded through the GPUI asset source; no icon library is required. User preference and system reduced-motion integration remain pending.

## Shared components

| Component in `src/ui/components.rs` | Contract / current consumer |
| --- | --- |
| `Button` | Primary, standard, quiet and destructive tones; an explicit state that carries the activation callback only when enabled or selected. Disabled Open files and Queue in the pending shell. Equal-width alternate labels use the maximum shaped text width plus padding and border. |
| `icon_action` | A supplemental glyph with a visible action label; no unlabeled icon-only controls yet. |
| `selector`, `selectable_row` | Wrap whole controls and use the button selection treatment. Workspace and queue consumers pending. |
| `heading`, `separator`, `status`, `inline_message`, `empty_state` | Concise headings and necessary information, with readable inline status and optional Details group. The pending shell consumes separator and empty state. |
| `pending`, `spinner`, `progress` | Real loading or progress only; no placeholder percentage. Background integration and real consumers pending. |
| `action_bar` | Wrap complete controls without shifting labels; shared bottom action area used by the pending shell. |

Construct a `Button::new` with a stable element ID, explicit tone and label. Set `state` to `ActionState::Enabled(Box::new(callback))` or `Selected` only when the owner permits activation; pass the current palette and window to `render`. Supply all alternate labels when switching between them must not change control width. Views do not recreate the button border or hover styles. The shell uses a single scrollable page with title, necessary unavailable state and disabled action bar. Intake, operation selection, queue navigation, real pending states and settings are not yet available in the native shell. No dummy actions or fake media jobs are provided to demonstrate controls.

## Copy, rows and feedback

Use explicit verbs and concise titles, no redundant subtitles, slogans or descriptions of obvious actions. Preserve actual counts, destinations, units, prerequisites, validation and consequences. Keep status labels short and provide full accessible reasons for failure; do not put raw logs or internal IDs into the primary UI. Long identity strings may ellipsize with full text accessible, but status and action labels remain readable in full.

Queue and batch headers and rows share a column layout. Align text left, numbers consistently and actions right. Vertically center the complete identity block, status and action groups. Keep status and inline Details on the same baseline and size the shared column for its widest supported content. Reflow whole groups at narrow widths or higher zoom, rather than clipping labels or breaking Details onto a new line. Never render empty disabled Details placeholders. Errors expose full diagnostics through a keyboard-accessible action, not hover alone.

For the GPUI batch port (U06), retain A02 behavior: `Edit shared settings` / `Apply settings`; per-file `Edit settings` / `Save changes` / `Cancel`; `Skip` / `Include` without losing configuration; `Custom settings` only for overrides; `Checking`, `Ready`, `Needs review`, `Incompatible`, `Skipped`, `Couldn't open`; `Add N jobs to queue` (singular for one). The right-aligned action pair uses a fixed slot for Edit settings and an equal-width Skip/Include slot based on the wider measured label. Keep a missing Edit settings slot empty, not interactive. Reflow the pair as a unit. Show actual counts once and avoid redundant blocked-count text. The latest retained batch behavior opens diagnostics from the clickable affected status (underlined on hover); retain that explicit interaction instead of adding a redundant Details action back to batch. Other diagnostic groups follow the inline Details contract. Full error reasons, atomic admission, inclusion and overrides remain owned by existing batch/queue state.

## Verification and limitations

Review actual reference and minimum windows, representative long labels, disabled/busy/selected/focused controls, stable dimensions and keyboard operation. Inspect Windows rendering at 100%, 125%, 150% and 200% DPI, including borders, fallback glyphs, weight selection and scroll reachability. Verify the packaged executable opens without installed Geist or network access. Check that animation runs only while needed and reduced motion uses static feedback. The Linux verification environment does not establish Windows DirectWrite, accessibility or DPI behavior. Native visual acceptance, font metadata/checksum and integration of the remaining controls are outstanding; do not claim a smoother renderer or complete GPUI workflow based on the palette and components alone.

use egui::{
    Align, Frame, InnerResponse, Layout, Margin, Rect, Response, ScrollArea, Sense, Sides, Style,
    TextStyle, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

use super::{fonts, theme};

/// Reference outer-window geometry selected by `.project/design/LAYOUT.md`.
pub const REFERENCE_WINDOW_SIZE: Vec2 = Vec2::new(730.0, 520.0);

/// Native window chrome accounted for by the selected reference geometry.
pub const WINDOW_CHROME_HEIGHT: f32 = 31.0;

/// Eframe's viewport size is the drawable content area, excluding native window chrome.
pub const REFERENCE_CONTENT_SIZE: Vec2 = Vec2::new(
    REFERENCE_WINDOW_SIZE.x,
    REFERENCE_WINDOW_SIZE.y - WINDOW_CHROME_HEIGHT,
);

/// Preserve the existing minimum drawable area while keeping it under one layout owner.
pub const MINIMUM_CONTENT_SIZE: Vec2 = Vec2::new(400.0, 260.0);

/// Quiet Focus content inset on every edge of the application surface.
pub const CONTENT_PADDING: i8 = 28;

/// Vertical inset inside the stable ready-operation action bar.
pub const ACTION_BAR_VERTICAL_PADDING: i8 = 10;

/// Maximum width for bounded sparse content at the reference window size after content insets.
///
/// File intake keeps this reference-width bound. Ready/configuration surfaces intentionally do not.
pub const MAX_CONTENT_WIDTH: f32 = REFERENCE_WINDOW_SIZE.x - 2.0 * CONTENT_PADDING as f32;

/// Minimum comfortable width for each field in a responsive two-field row.
pub const FIELD_PAIR_MIN_COLUMN_WIDTH: f32 = 180.0;

/// Stable width for one editable pixel-boundary control.
pub const PIXEL_DIMENSION_CONTROL_WIDTH: f32 = 96.0;

/// Mouse/keyboard adjustment increment for editable pixel boundaries.
pub const PIXEL_DIMENSION_STEP: f64 = 2.0;

/// Stable width for the numeric encoder-quality control.
pub const ENCODER_QUALITY_CONTROL_WIDTH: f32 = 96.0;

/// Encoder quality values change in whole-number steps.
pub const ENCODER_QUALITY_STEP: f64 = 1.0;

/// Minimum useful inline width for the editable output filename before the row stacks.
pub const OUTPUT_FILENAME_MIN_INLINE_WIDTH: f32 = 180.0;

/// Minimum title/metadata width retained beside an inline Audio Mix gain control.
pub const TRACK_TEXT_MIN_INLINE_WIDTH: f32 = 240.0;

/// Minimum useful width retained for the flexible source/output identity region.
pub const QUEUE_IDENTITY_MIN_WIDTH: f32 = 180.0;

/// Bounded queue operation and state widths keep the shared wide-row rhythm stable.
pub const QUEUE_OPERATION_WIDTH: f32 = 104.0;
pub const QUEUE_STATE_WIDTH: f32 = 78.0;

/// Shared wide-row height vertically centers bounded cells while identity uses two lines.
pub const QUEUE_ROW_HEIGHT: f32 = 44.0;

/// Batch rows share Queue's two-line identity rhythm while reserving stable status and actions.
pub const BATCH_IDENTITY_MIN_WIDTH: f32 = 180.0;
pub const BATCH_STATE_WIDTH: f32 = 150.0;
pub const BATCH_ROW_HEIGHT: f32 = 44.0;

/// Useful source identity width retained beside workspace source actions before compact metadata
/// is omitted at narrow widths.
pub const WORKSPACE_METADATA_MIN_WIDTH: f32 = 220.0;

/// Narrow focal width used by Audio Mix running and terminal surfaces.
pub const FOCUS_CONTENT_MAX_WIDTH: f32 = 470.0;

/// Reference minimum focal-region height from the selected Quiet Focus surface recipe.
pub const FOCUS_CONTENT_MIN_HEIGHT: f32 = 230.0;

/// Failure diagnostics remain subordinate and bounded inside terminal surfaces.
pub const DIAGNOSTIC_MAX_HEIGHT: f32 = 80.0;

/// Progress tracks are deliberately thin and rectangular.
pub const PROGRESS_TRACK_HEIGHT: f32 = 4.0;

/// Paths and filenames stay on one line and truncate rather than forcing actions off-screen.
pub const PATH_TEXT_OVERFLOW: TextOverflowPolicy = TextOverflowPolicy::Ellipsis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionLayout {
    Horizontal,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldPairLayout {
    Columns,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRowLayout {
    Inline,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackRowLayout {
    Inline,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueRowLayout {
    Inline,
    Stacked,
}

pub type BatchRowLayout = QueueRowLayout;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchRowPlan {
    pub layout: BatchRowLayout,
    pub row_height: f32,
    pub identity_width: f32,
    pub state_width: f32,
    pub action_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchRowColumns {
    pub identity: Rect,
    pub state: Rect,
    pub actions: Rect,
}

impl BatchRowPlan {
    pub fn columns(self, row: Rect) -> BatchRowColumns {
        debug_assert_eq!(self.layout, BatchRowLayout::Inline);
        let gap = theme::SPACING.related;
        let identity = Rect::from_min_size(row.min, vec2(self.identity_width, row.height()));
        let state = Rect::from_min_size(
            identity.right_top() + vec2(gap, 0.0),
            vec2(self.state_width, row.height()),
        );
        let actions = Rect::from_min_size(
            state.right_top() + vec2(gap, 0.0),
            vec2(self.action_width, row.height()),
        );
        debug_assert!((actions.right() - row.right()).abs() <= 0.5);
        BatchRowColumns {
            identity,
            state,
            actions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueueRowPlan {
    pub layout: QueueRowLayout,
    pub row_height: f32,
    pub identity_width: f32,
    pub operation_width: f32,
    pub state_width: f32,
    pub action_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueueRowColumns {
    pub identity: Rect,
    pub operation: Rect,
    pub state: Rect,
    pub actions: Option<Rect>,
}

impl QueueRowPlan {
    /// Resolve the shared wide-row plan into exact cells. Callers allocate the row once and render
    /// each cell inside these rectangles so child content cannot move a later column.
    pub fn columns(self, row: Rect) -> QueueRowColumns {
        debug_assert_eq!(self.layout, QueueRowLayout::Inline);
        let height = row.height();
        let gap = theme::SPACING.related;
        let identity = Rect::from_min_size(row.min, vec2(self.identity_width, height));
        let operation = Rect::from_min_size(
            identity.right_top() + vec2(gap, 0.0),
            vec2(self.operation_width, height),
        );
        let state = Rect::from_min_size(
            operation.right_top() + vec2(gap, 0.0),
            vec2(self.state_width, height),
        );
        let actions = (self.action_width > 0.0).then(|| {
            Rect::from_min_size(
                state.right_top() + vec2(gap, 0.0),
                vec2(self.action_width, height),
            )
        });

        debug_assert!(
            (actions.map_or(state.right(), |actions| actions.right()) - row.right()).abs() <= 0.5
        );
        QueueRowColumns {
            identity,
            operation,
            state,
            actions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackRowPlan {
    pub layout: TrackRowLayout,
    pub row_height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputRowPlan {
    pub layout: OutputRowLayout,
    pub filename_width: f32,
    pub leading_width: Option<f32>,
}

impl OutputRowPlan {
    pub fn flow_height(self) -> f32 {
        match self.layout {
            OutputRowLayout::Inline => theme::COMPACT_CONTROL_HEIGHT,
            OutputRowLayout::Stacked => {
                let control_count = if self.leading_width.is_some() {
                    3.0
                } else {
                    2.0
                };
                let gap_count = control_count - 1.0;
                control_count * theme::COMPACT_CONTROL_HEIGHT + gap_count * theme::SPACING.compact
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntakeLayout {
    Distributed,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntakeLayoutPlan {
    pub layout: IntakeLayout,
    pub region_gap: f32,
}

pub struct OperationSurfaceResponse<Content, Action> {
    pub content: Content,
    pub action: Action,
    pub content_viewport: Rect,
    pub content_gutter: Rect,
    pub action_bar: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverflowPolicy {
    Ellipsis,
}

/// Apply no outer inset so an operation-content scrollbar can own the central-panel edge.
pub fn root_frame(style: &Style) -> Frame {
    Frame::central_panel(style)
}

/// Apply the normal Quiet Focus inset to surfaces that do not use the ready-operation shell.
pub fn padded_content<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let available = safe_available_size(ui.available_size());
    Frame::NONE
        .inner_margin(Margin::same(CONTENT_PADDING))
        .show(ui, |ui| {
            ui.set_min_size(vec2(
                (available.x - f32::from(CONTENT_PADDING) * 2.0).max(0.0),
                (available.y - f32::from(CONTENT_PADDING) * 2.0).max(0.0),
            ));
            add_contents(ui)
        })
}

/// Apply normal top and horizontal padding to persistent workspace chrome while leaving the
/// operation body and its scrollbar to own the remaining full-width rectangle.
pub fn workspace_chrome<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let available_width = safe_dimension(ui.available_width());
    Frame::NONE
        .inner_margin(Margin {
            left: CONTENT_PADDING,
            right: CONTENT_PADDING,
            top: CONTENT_PADDING,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_min_width((available_width - f32::from(CONTENT_PADDING) * 2.0).max(0.0));
            add_contents(ui)
        })
}

/// Apply only the shared horizontal gutter to workspace-level feedback outside operation content.
pub fn content_gutter<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let available_width = safe_dimension(ui.available_width());
    Frame::NONE
        .inner_margin(Margin {
            left: CONTENT_PADDING,
            right: CONTENT_PADDING,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_min_width((available_width - f32::from(CONTENT_PADDING) * 2.0).max(0.0));
            add_contents(ui)
        })
}

/// Align a collection title/summary and its trailing controls to one compact header axis.
/// Queue establishes this pattern; Batch can reuse it when its presentation is corrected.
pub fn collection_header<Leading, Actions>(
    ui: &mut Ui,
    add_leading: impl FnOnce(&mut Ui) -> Leading,
    add_actions: impl FnOnce(&mut Ui) -> Actions,
) {
    Sides::new()
        .height(theme::COMPACT_CONTROL_HEIGHT)
        .spacing(theme::SPACING.related)
        .shrink_left()
        .truncate()
        .show(
            ui,
            |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), add_leading);
            },
            |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), add_actions);
            },
        );
}

/// Render a collection title and summary on one typographic baseline.
///
/// Centering separate egui labels aligns their logical line boxes, not their glyph baselines. Keep
/// the two galleys in one allocation and derive their relative y positions from the first glyph in
/// each installed font.
pub fn collection_heading(ui: &mut Ui, title: &str, summary: &str) -> Response {
    let title_color = ui.visuals().text_color();
    let title_galley = ui.painter().layout_no_wrap(
        title.to_owned(),
        TextStyle::Heading.resolve(ui.style()),
        title_color,
    );
    let summary_galley = ui.painter().layout_no_wrap(
        summary.to_owned(),
        fonts::technical_text_style().resolve(ui.style()),
        theme::SECONDARY_FAINT,
    );

    let glyph_baseline = |galley: &egui::Galley| {
        let row = galley.rows.first().expect("single-line text has one row");
        row.pos.y
            + row
                .glyphs
                .first()
                .expect("collection heading text is non-empty")
                .pos
                .y
    };
    let title_baseline = glyph_baseline(&title_galley);
    let summary_baseline = glyph_baseline(&summary_galley);
    let summary_y_from_title = title_baseline - summary_baseline;
    let content_top = summary_y_from_title.min(0.0);
    let content_bottom = title_galley
        .size()
        .y
        .max(summary_y_from_title + summary_galley.size().y);
    let content_size = vec2(
        title_galley.size().x + theme::SPACING.compact + summary_galley.size().x,
        content_bottom - content_top,
    );
    let (rect, response) = ui.allocate_exact_size(content_size, Sense::hover());
    let title_position = pos2(rect.left(), rect.top() - content_top);
    let summary_position = pos2(
        title_position.x + title_galley.size().x + theme::SPACING.compact,
        title_position.y + summary_y_from_title,
    );
    ui.painter()
        .galley(title_position, title_galley, title_color);
    ui.painter()
        .galley(summary_position, summary_galley, theme::SECONDARY_FAINT);
    response.widget_info(|| {
        WidgetInfo::labeled(WidgetType::Label, true, format!("{title}, {summary}"))
    });
    response
}

/// Fade a replacement operation surface without moving or resizing its layout.
pub fn surface_fade<R>(
    ui: &mut Ui,
    surface_key: egui::Id,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let owner_id = ui.make_persistent_id("operation-surface-fade");
    let animation_id = owner_id.with("opacity");
    let duration = theme::motion_durations(ui.ctx()).surface_enter;
    let previous_key = ui.ctx().data(|data| data.get_temp::<egui::Id>(owner_id));
    let changed = previous_key != Some(surface_key);

    if changed {
        ui.ctx()
            .data_mut(|data| data.insert_temp(owner_id, surface_key));
    }

    let opacity = if duration <= 0.0 {
        1.0
    } else {
        if changed {
            let _ = ui.ctx().animate_value_with_time(
                animation_id,
                theme::SURFACE_ENTER_START_OPACITY,
                0.0,
            );
        }
        ui.ctx()
            .animate_value_with_time(animation_id, 1.0, duration)
    };

    ui.scope(|ui| {
        ui.multiply_opacity(opacity);
        add_contents(ui)
    })
    .inner
}

/// Render a ready operation as one full-width scrolling content region plus a stable bottom action
/// bar. Normal horizontal padding lives inside the scroll area, keeping the scrollbar lane at the
/// central content edge and preventing operation-level nested scrolling.
pub fn operation_surface<Content, Action>(
    ui: &mut Ui,
    add_content: impl FnOnce(&mut Ui) -> Content,
    add_action: impl FnOnce(&mut Ui) -> Action,
) -> OperationSurfaceResponse<Content, Action> {
    operation_surface_with_action_rows(ui, 1, add_content, add_action)
}

/// Render an operation surface whose responsive bottom controls require multiple rows.
/// Existing ready surfaces use one row; queue controls may request two at compact widths.
pub fn operation_surface_with_action_rows<Content, Action>(
    ui: &mut Ui,
    action_rows: usize,
    add_content: impl FnOnce(&mut Ui) -> Content,
    add_action: impl FnOnce(&mut Ui) -> Action,
) -> OperationSurfaceResponse<Content, Action> {
    let available = safe_available_size(
        ui.available_rect_before_wrap()
            .intersect(ui.clip_rect())
            .size(),
    );
    let previous_item_spacing_y = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let action_rows = action_rows.max(1) as f32;
    let action_height = theme::PRIMARY_CONTROL_HEIGHT * action_rows
        + theme::SPACING.compact * (action_rows - 1.0)
        + f32::from(ACTION_BAR_VERTICAL_PADDING) * 2.0
        + theme::SEPARATOR_WIDTH;
    let content_height = (available.y - action_height).max(0.0);
    let content_gutter = std::cell::Cell::new(Rect::NOTHING);

    let (content_viewport, _) =
        ui.allocate_exact_size(vec2(available.x, content_height), egui::Sense::hover());
    let mut content_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(content_viewport)
            .layout(Layout::top_down(Align::LEFT)),
    );
    let content = ScrollArea::vertical()
        .id_salt("ready-operation-content")
        .auto_shrink([false, false])
        .show(&mut content_ui, |ui| {
            let scroll_width = safe_dimension(ui.available_width());
            Frame::NONE
                .inner_margin(Margin {
                    left: CONTENT_PADDING,
                    right: CONTENT_PADDING,
                    top: 6,
                    bottom: CONTENT_PADDING,
                })
                .show(ui, |ui| {
                    content_gutter.set(ui.max_rect());
                    ui.set_min_width((scroll_width - f32::from(CONTENT_PADDING) * 2.0).max(0.0));
                    add_content(ui)
                })
                .inner
        })
        .inner;

    let (action_bar, _) =
        ui.allocate_exact_size(vec2(available.x, action_height), egui::Sense::hover());
    let action_content = Rect::from_min_max(
        action_bar.min
            + vec2(
                f32::from(CONTENT_PADDING),
                f32::from(ACTION_BAR_VERTICAL_PADDING) + theme::SEPARATOR_WIDTH,
            ),
        action_bar.max
            - vec2(
                f32::from(CONTENT_PADDING),
                f32::from(ACTION_BAR_VERTICAL_PADDING),
            ),
    );
    let mut action_bar_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(action_bar)
            .layout(Layout::top_down(Align::LEFT)),
    );
    action_bar_ui.painter().line_segment(
        [action_bar.left_top(), action_bar.right_top()],
        egui::Stroke::new(theme::SEPARATOR_WIDTH, theme::SEPARATOR),
    );
    let action_layout = if action_rows > 1.0 {
        Layout::top_down(Align::Max)
    } else {
        Layout::right_to_left(Align::Center)
    };
    let mut action_ui = action_bar_ui.new_child(
        UiBuilder::new()
            .max_rect(action_content)
            .layout(action_layout),
    );
    let action = add_action(&mut action_ui);
    ui.spacing_mut().item_spacing.y = previous_item_spacing_y;

    OperationSurfaceResponse {
        content,
        action,
        content_viewport,
        content_gutter: content_gutter.get(),
        action_bar,
    }
}

/// Center a sparse focal group within a bounded content column using egui layout flow.
///
/// The group receives the current available height so its vertical layout can center naturally.
/// No screen coordinates are computed or stored.
pub fn centered_content<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let available = safe_available_size(ui.available_size());
    let content_width = bounded_content_width(available.x);

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.allocate_ui_with_layout(
            vec2(content_width, available.y),
            Layout::top_down(Align::Center).with_main_align(Align::Center),
            add_contents,
        )
    })
    .inner
}

/// Arrange file intake as top, middle, and bottom regions when height permits.
///
/// The caller supplies presentation-only region heights so each region renders exactly once. Stable
/// controls use deterministic metrics while wrapped copy can be measured directly from text. Remaining
/// vertical space is distributed evenly between them, keeping the title near the top, the intake
/// action around the visual middle, and the mode selector at the bottom. When the shared section
/// gaps no longer fit, the surface collapses to compact top-down flow inside a vertical scroll area.
pub fn intake_content(
    ui: &mut Ui,
    region_heights: [f32; 3],
    show_top: impl FnOnce(&mut Ui),
    show_middle: impl FnOnce(&mut Ui),
    show_bottom: impl FnOnce(&mut Ui),
) -> IntakeLayoutPlan {
    let available = safe_available_size(ui.available_size());
    let content_width = bounded_content_width(available.x);
    let plan = intake_layout_plan(available.y, region_heights);

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.allocate_ui_with_layout(
            vec2(content_width, available.y),
            Layout::top_down(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                match plan.layout {
                    IntakeLayout::Distributed => {
                        show_top(ui);
                        ui.add_space(plan.region_gap);
                        show_middle(ui);
                        ui.add_space(plan.region_gap);
                        show_bottom(ui);
                    }
                    IntakeLayout::Compact => {
                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                show_top(ui);
                                ui.add_space(theme::SPACING.section);
                                show_middle(ui);
                                ui.add_space(theme::SPACING.section);
                                show_bottom(ui);
                            });
                    }
                }
            },
        );
    });

    plan
}

/// Choose the three-region intake composition only while shared section gaps still fit.
pub fn intake_layout_plan(available_height: f32, region_heights: [f32; 3]) -> IntakeLayoutPlan {
    let available_height = safe_dimension(available_height);
    let total_region_height = region_heights.into_iter().map(safe_dimension).sum::<f32>();
    let minimum_gap = theme::SPACING.section;
    let required_height = total_region_height + minimum_gap * 2.0;

    if required_height <= available_height {
        IntakeLayoutPlan {
            layout: IntakeLayout::Distributed,
            region_gap: ((available_height - total_region_height) * 0.5).max(minimum_gap),
        }
    } else {
        IntakeLayoutPlan {
            layout: IntakeLayout::Compact,
            region_gap: minimum_gap,
        }
    }
}

/// Center a ready/configuration feature surface while keeping its contents top-down and left aligned.
///
/// Ready surfaces consume the current padded viewport width. The surrounding content frame owns
/// responsive gutters, while this helper avoids any reference-size content ceiling.
pub fn task_content<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let available = safe_available_size(ui.available_size());
    let content_width = available.x;

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.allocate_ui_with_layout(
            vec2(content_width, available.y),
            Layout::top_down(Align::LEFT),
            add_contents,
        )
    })
    .inner
}

/// Result of laying out a narrow running/result focal composition.
///
/// [`content_size`](Self::content_size) is measured from the current rendered composition before
/// placement, while [`viewport_size`](Self::viewport_size) is the visible focal region. This keeps
/// overflow decisions content-aware without turning the Quiet Focus reference minimum into a
/// global threshold.
pub struct FocusContentResponse<R> {
    pub inner: R,
    pub content_size: Vec2,
    pub viewport_size: Vec2,
}

impl<R> FocusContentResponse<R> {
    /// Whether the current composition exceeds the visible focal viewport.
    pub fn is_scrollable(&self) -> bool {
        self.content_size.y > self.viewport_size.y + f32::EPSILON
    }
}

/// Center a narrow running/result surface as one sparse focal composition.
///
/// A sizing pass measures the current composition at the same bounded width, including wrapped
/// copy and expanded bounded diagnostics. Callers must keep that sizing invocation presentation-only
/// because the closure runs again for the real render. If the measured height fits, the real surface keeps
/// vertically centered top-down flow with no outer scroll area. If it does not fit, the real
/// surface uses a vertical [`ScrollArea`] so terminal actions remain reachable.
///
/// `FOCUS_CONTENT_MIN_HEIGHT` remains the selected Quiet Focus reference minimum. Actual rendered
/// content, not that metric, is authoritative for overflow.
pub fn focus_content<R>(
    ui: &mut Ui,
    mut add_contents: impl FnMut(&mut Ui) -> R,
) -> FocusContentResponse<R> {
    let available = safe_available_size(ui.available_size());
    let content_width = bounded_content_width(available.x).min(FOCUS_CONTENT_MAX_WIDTH);
    let measure_rect = Rect::from_min_size(ui.cursor().min, vec2(content_width, available.y));
    let mut measure_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("focus_content_measure")
            .max_rect(measure_rect)
            .layout(Layout::top_down(Align::LEFT))
            .sizing_pass()
            .invisible(),
    );
    let _ = add_contents(&mut measure_ui);
    let measured_size = safe_available_size(measure_ui.min_rect().size());
    let overflow = measured_size.y > available.y + f32::EPSILON;

    let rendered = ui
        .with_layout(Layout::top_down(Align::Center), |ui| {
            ui.allocate_ui_with_layout(
                vec2(content_width, available.y),
                Layout::top_down(Align::LEFT),
                |ui| {
                    if overflow {
                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, &mut add_contents)
                            .inner
                    } else {
                        let top_space = ((available.y - measured_size.y) * 0.5).max(0.0);
                        ui.add_space(top_space);
                        add_contents(ui)
                    }
                },
            )
        })
        .inner;

    FocusContentResponse {
        inner: rendered.inner,
        content_size: measured_size,
        viewport_size: vec2(content_width, available.y),
    }
}

/// Choose an inline action row only when every intrinsic action width plus shared gaps fits.
pub fn action_layout(available_width: f32, action_widths: &[f32]) -> ActionLayout {
    let available_width = safe_dimension(available_width);
    let total_action_width = action_widths
        .iter()
        .copied()
        .map(safe_dimension)
        .sum::<f32>();
    let gap_count = action_widths.len().saturating_sub(1) as f32;
    let required_width = total_action_width + gap_count * theme::SPACING.related;

    if required_width.is_finite() && required_width <= available_width {
        ActionLayout::Horizontal
    } else {
        ActionLayout::Stacked
    }
}

pub fn workspace_header_shows_metadata(available_width: f32, source_action_widths: &[f32]) -> bool {
    let action_width = source_action_widths
        .iter()
        .copied()
        .map(safe_dimension)
        .sum::<f32>()
        + source_action_widths.len().saturating_sub(1) as f32 * theme::INLINE_ACTION_GAP;
    safe_dimension(available_width)
        >= action_width + theme::SOURCE_HEADER_GAP + WORKSPACE_METADATA_MIN_WIDTH
}

pub fn workspace_switcher_fits_one_line(available_width: f32, control_widths: &[f32]) -> bool {
    matches!(
        action_layout(available_width, control_widths),
        ActionLayout::Horizontal
    )
}

/// Keep paired task fields in columns only while both retain a comfortable readable width.
pub fn field_pair_layout(available_width: f32) -> FieldPairLayout {
    let available_width = safe_dimension(available_width);
    let required_width = FIELD_PAIR_MIN_COLUMN_WIDTH * 2.0 + theme::SPACING.related;

    if available_width >= required_width {
        FieldPairLayout::Columns
    } else {
        FieldPairLayout::Stacked
    }
}

pub fn queue_row_plan(available_width: f32, action_width: f32) -> QueueRowPlan {
    let available_width = safe_dimension(available_width);
    let action_width = safe_dimension(action_width);
    let gap_count = if action_width > 0.0 { 3.0 } else { 2.0 };
    let fixed_width = QUEUE_OPERATION_WIDTH
        + QUEUE_STATE_WIDTH
        + action_width
        + gap_count * theme::SPACING.related;
    let identity_width = (available_width - fixed_width).max(0.0);
    let layout = if identity_width >= QUEUE_IDENTITY_MIN_WIDTH {
        QueueRowLayout::Inline
    } else {
        QueueRowLayout::Stacked
    };

    QueueRowPlan {
        layout,
        row_height: QUEUE_ROW_HEIGHT,
        identity_width,
        operation_width: QUEUE_OPERATION_WIDTH,
        state_width: QUEUE_STATE_WIDTH,
        action_width,
    }
}

pub fn batch_row_plan(available_width: f32, action_width: f32) -> BatchRowPlan {
    let available_width = safe_dimension(available_width);
    let action_width = safe_dimension(action_width);
    let fixed_width = BATCH_STATE_WIDTH + action_width + 2.0 * theme::SPACING.related;
    let identity_width = (available_width - fixed_width).max(0.0);
    let layout = if identity_width >= BATCH_IDENTITY_MIN_WIDTH {
        BatchRowLayout::Inline
    } else {
        BatchRowLayout::Stacked
    };
    BatchRowPlan {
        layout,
        row_height: BATCH_ROW_HEIGHT,
        identity_width,
        state_width: BATCH_STATE_WIDTH,
        action_width,
    }
}

/// Intrinsic bounds of an action group for the selected responsive arrangement.
///
/// The group size is derived only from intrinsic action sizes and the supplied shared gap. It does
/// not depend on viewport coordinates.
pub fn action_group_size(action_sizes: &[Vec2], arrangement: ActionLayout, gap: f32) -> Vec2 {
    let gap = safe_dimension(gap);
    let gap_count = action_sizes.len().saturating_sub(1) as f32;

    match arrangement {
        ActionLayout::Horizontal => vec2(
            action_sizes
                .iter()
                .map(|size| safe_dimension(size.x))
                .sum::<f32>()
                + gap_count * gap,
            action_sizes
                .iter()
                .map(|size| safe_dimension(size.y))
                .fold(0.0, f32::max),
        ),
        ActionLayout::Stacked => vec2(
            action_sizes
                .iter()
                .map(|size| safe_dimension(size.x))
                .fold(0.0, f32::max),
            action_sizes
                .iter()
                .map(|size| safe_dimension(size.y))
                .sum::<f32>()
                + gap_count * gap,
        ),
    }
}

/// Center an intrinsic action group inside the current content width without consuming remaining
/// vertical space.
///
/// Horizontal rows are centered as intrinsic child groups. Stacked groups center each action on
/// the horizontal cross axis. In both cases the allocated region is exactly the intrinsic group
/// height, so following content stays in normal top-down flow.
pub fn centered_action_group<R>(
    ui: &mut Ui,
    action_sizes: &[Vec2],
    arrangement: ActionLayout,
    gap: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let group_size = action_group_size(action_sizes, arrangement, gap);
    let flow = match arrangement {
        ActionLayout::Horizontal => Layout::left_to_right(Align::Center),
        ActionLayout::Stacked => Layout::top_down(Align::Center),
    };

    ui.vertical_centered(|ui| {
        ui.allocate_ui_with_layout(group_size, flow, |ui| {
            match arrangement {
                ActionLayout::Horizontal => ui.spacing_mut().item_spacing.x = safe_dimension(gap),
                ActionLayout::Stacked => ui.spacing_mut().item_spacing.y = safe_dimension(gap),
            }
            add_contents(ui)
        })
        .inner
    })
}

/// Bound readable task content to the Quiet Focus reference width without returning invalid sizes.
pub fn bounded_content_width(available_width: f32) -> f32 {
    safe_dimension(available_width).min(MAX_CONTENT_WIDTH)
}

/// Plan the shared editable-output row from the current content width and intrinsic fixed controls.
///
/// Inline rows reserve stable space for an optional leading control and Browse, leaving the
/// filename as the only flexible element. When that would squeeze the filename below its useful
/// width, the controls stack in leading/name/Browse order without shrinking hit targets.
pub fn output_row_plan(
    available_width: f32,
    leading_width: Option<f32>,
    browse_width: f32,
) -> OutputRowPlan {
    let available_width = safe_dimension(available_width);
    let leading_width = leading_width.map(safe_dimension);
    let browse_width = safe_dimension(browse_width);
    let gap_count = if leading_width.is_some() { 2.0 } else { 1.0 };
    let fixed_width =
        leading_width.unwrap_or(0.0) + browse_width + gap_count * theme::SPACING.compact;
    let inline_filename_width = (available_width - fixed_width).max(0.0);

    if inline_filename_width >= OUTPUT_FILENAME_MIN_INLINE_WIDTH {
        OutputRowPlan {
            layout: OutputRowLayout::Inline,
            filename_width: inline_filename_width,
            leading_width,
        }
    } else {
        OutputRowPlan {
            layout: OutputRowLayout::Stacked,
            filename_width: available_width,
            leading_width,
        }
    }
}

/// Keep the gain beside track identity while the title remains useful, then move it below.
pub fn track_row_plan(available_width: f32, gain_width: f32) -> TrackRowPlan {
    let available_width = safe_dimension(available_width);
    let gain_width = safe_dimension(gain_width);
    let inline_fixed_width = theme::SELECTABLE_ROW_HORIZONTAL_PADDING
        + theme::SELECTABLE_ROW_INDICATOR_COLUMN
        + theme::SELECTABLE_ROW_CONTENT_GAP
        + theme::SPACING.compact
        + gain_width
        + theme::SELECTABLE_ROW_HORIZONTAL_PADDING;

    if available_width - inline_fixed_width >= TRACK_TEXT_MIN_INLINE_WIDTH {
        TrackRowPlan {
            layout: TrackRowLayout::Inline,
            row_height: theme::SELECTABLE_ROW_MIN_HEIGHT,
        }
    } else {
        TrackRowPlan {
            layout: TrackRowLayout::Stacked,
            row_height: theme::SELECTABLE_ROW_MIN_HEIGHT
                + theme::SPACING.compact
                + theme::COMPACT_CONTROL_HEIGHT,
        }
    }
}

fn safe_available_size(size: Vec2) -> Vec2 {
    vec2(safe_dimension(size.x), safe_dimension(size.y))
}

fn safe_dimension(value: f32) -> f32 {
    if value.is_nan() || value <= 0.0 {
        0.0
    } else if value.is_infinite() {
        f32::MAX
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use egui::{Rect, Vec2, pos2};

    use super::{
        ACTION_BAR_VERTICAL_PADDING, ActionLayout, BATCH_IDENTITY_MIN_WIDTH, BATCH_ROW_HEIGHT,
        BATCH_STATE_WIDTH, BatchRowLayout, CONTENT_PADDING, DIAGNOSTIC_MAX_HEIGHT,
        ENCODER_QUALITY_CONTROL_WIDTH, ENCODER_QUALITY_STEP, FIELD_PAIR_MIN_COLUMN_WIDTH,
        FOCUS_CONTENT_MAX_WIDTH, FOCUS_CONTENT_MIN_HEIGHT, FieldPairLayout, IntakeLayout,
        MAX_CONTENT_WIDTH, MINIMUM_CONTENT_SIZE, OUTPUT_FILENAME_MIN_INLINE_WIDTH, OutputRowLayout,
        PATH_TEXT_OVERFLOW, PIXEL_DIMENSION_CONTROL_WIDTH, PIXEL_DIMENSION_STEP,
        PROGRESS_TRACK_HEIGHT, QUEUE_IDENTITY_MIN_WIDTH, QUEUE_OPERATION_WIDTH, QUEUE_ROW_HEIGHT,
        QUEUE_STATE_WIDTH, QueueRowLayout, REFERENCE_CONTENT_SIZE, REFERENCE_WINDOW_SIZE,
        TRACK_TEXT_MIN_INLINE_WIDTH, TextOverflowPolicy, TrackRowLayout, WINDOW_CHROME_HEIGHT,
        WORKSPACE_METADATA_MIN_WIDTH, action_group_size, action_layout, batch_row_plan,
        bounded_content_width, centered_action_group, field_pair_layout, focus_content,
        intake_content, intake_layout_plan, operation_surface, output_row_plan, queue_row_plan,
        surface_fade, track_row_plan, workspace_header_shows_metadata,
        workspace_switcher_fits_one_line,
    };
    use crate::ui::theme;

    #[test]
    fn quiet_focus_reference_geometry_is_centralized() {
        assert_eq!(REFERENCE_WINDOW_SIZE, Vec2::new(730.0, 520.0));
        assert_eq!(WINDOW_CHROME_HEIGHT, 31.0);
        assert_eq!(REFERENCE_CONTENT_SIZE, Vec2::new(730.0, 489.0));
        assert_eq!(MINIMUM_CONTENT_SIZE, Vec2::new(400.0, 260.0));
        assert_eq!(CONTENT_PADDING, 28);
        assert_eq!(ACTION_BAR_VERTICAL_PADDING, 10);
        assert_eq!(MAX_CONTENT_WIDTH, 674.0);
        assert_eq!(FIELD_PAIR_MIN_COLUMN_WIDTH, 180.0);
        assert_eq!(PIXEL_DIMENSION_CONTROL_WIDTH, 96.0);
        assert_eq!(PIXEL_DIMENSION_STEP, 2.0);
        assert_eq!(ENCODER_QUALITY_CONTROL_WIDTH, 96.0);
        assert_eq!(QUEUE_IDENTITY_MIN_WIDTH, 180.0);
        assert_eq!(QUEUE_OPERATION_WIDTH, 104.0);
        assert_eq!(QUEUE_STATE_WIDTH, 78.0);
        assert_eq!(QUEUE_ROW_HEIGHT, 44.0);
        assert_eq!(BATCH_IDENTITY_MIN_WIDTH, 180.0);
        assert_eq!(BATCH_STATE_WIDTH, 150.0);
        assert_eq!(BATCH_ROW_HEIGHT, 44.0);
        assert_eq!(ENCODER_QUALITY_STEP, 1.0);
        assert_eq!(OUTPUT_FILENAME_MIN_INLINE_WIDTH, 180.0);
        assert_eq!(WORKSPACE_METADATA_MIN_WIDTH, 220.0);
        assert_eq!(FOCUS_CONTENT_MAX_WIDTH, 470.0);
        assert_eq!(FOCUS_CONTENT_MIN_HEIGHT, 230.0);
        assert_eq!(DIAGNOSTIC_MAX_HEIGHT, 80.0);
        assert_eq!(theme::PRIMARY_CONTROL_HEIGHT, 31.0);
        assert_eq!(theme::COMPACT_CONTROL_HEIGHT, 30.0);
        assert_eq!(theme::UPSCALE_TARGET_ROW_MIN_HEIGHT, 37.0);
        assert_eq!(theme::SPACING.compact, 7.0);
        assert_eq!(theme::SPACING.related, 11.0);
        assert_eq!(theme::SPACING.section, 24.0);
        assert_eq!(theme::SPACING.spacious, 40.0);
        assert_eq!(theme::SOURCE_HEADER_GAP, 14.0);
        assert_eq!(theme::INLINE_ACTION_GAP, 10.0);
        assert_eq!(PROGRESS_TRACK_HEIGHT, 4.0);
    }

    #[test]
    fn queue_rows_keep_bounded_cells_and_reflow_before_identity_becomes_unreadable() {
        let action_width = 72.0;
        let wide = queue_row_plan(620.0, action_width);
        assert_eq!(wide.layout, QueueRowLayout::Inline);
        assert_eq!(wide.operation_width, QUEUE_OPERATION_WIDTH);
        assert_eq!(wide.state_width, QUEUE_STATE_WIDTH);
        assert_eq!(wide.action_width, action_width);
        assert!(wide.identity_width >= QUEUE_IDENTITY_MIN_WIDTH);

        let narrow = queue_row_plan(420.0, action_width);
        assert_eq!(narrow.layout, QueueRowLayout::Stacked);
        assert!(narrow.identity_width < QUEUE_IDENTITY_MIN_WIDTH);
        assert_eq!(
            queue_row_plan(f32::NAN, action_width).layout,
            QueueRowLayout::Stacked
        );
    }

    #[test]
    fn batch_rows_keep_shared_bounded_cells_and_stack_at_narrow_width() {
        let action_width = 128.0;
        let wide = batch_row_plan(620.0, action_width);
        assert_eq!(wide.layout, BatchRowLayout::Inline);
        assert_eq!(wide.state_width, BATCH_STATE_WIDTH);
        assert_eq!(wide.action_width, action_width);
        assert!(wide.identity_width >= BATCH_IDENTITY_MIN_WIDTH);

        let row = Rect::from_min_size(pos2(8.0, 12.0), Vec2::new(620.0, BATCH_ROW_HEIGHT));
        let columns = wide.columns(row);
        assert_eq!(columns.identity.left(), row.left());
        assert!((columns.actions.right() - row.right()).abs() <= 0.5);

        assert_eq!(
            batch_row_plan(420.0, action_width).layout,
            BatchRowLayout::Stacked
        );
    }

    #[test]
    fn inline_queue_plan_resolves_one_exact_shared_column_geometry() {
        let plan = queue_row_plan(620.0, 144.0);
        assert_eq!(plan.layout, QueueRowLayout::Inline);
        let row = Rect::from_min_size(pos2(17.0, 23.0), Vec2::new(620.0, plan.row_height));
        let columns = plan.columns(row);

        assert_eq!(columns.identity.left(), row.left());
        assert_eq!(columns.identity.width(), plan.identity_width);
        assert_eq!(columns.operation.width(), QUEUE_OPERATION_WIDTH);
        assert_eq!(columns.state.width(), QUEUE_STATE_WIDTH);
        assert_eq!(columns.actions.unwrap().width(), plan.action_width);
        assert_eq!(columns.actions.unwrap().right(), row.right());
        assert_eq!(
            columns.operation.left() - columns.identity.right(),
            theme::SPACING.related
        );
        assert_eq!(
            columns.state.left() - columns.operation.right(),
            theme::SPACING.related
        );
        assert_eq!(
            columns.actions.unwrap().left() - columns.state.right(),
            theme::SPACING.related
        );
    }

    #[test]
    fn workspace_header_drops_metadata_before_source_actions_cramp() {
        let source_action_widths = [72.0, 85.0];

        assert!(workspace_header_shows_metadata(
            564.0,
            &source_action_widths
        ));
        assert!(!workspace_header_shows_metadata(
            344.0,
            &source_action_widths
        ));
    }

    #[test]
    fn workspace_switcher_uses_one_line_only_when_intrinsic_controls_fit() {
        let operation_widths = [84.0, 72.0, 65.0, 104.0, 118.0];

        assert!(workspace_switcher_fits_one_line(564.0, &operation_widths));
        assert!(!workspace_switcher_fits_one_line(344.0, &operation_widths));
    }

    #[test]
    fn reference_intake_height_preserves_three_distinct_regions() {
        let available_height = REFERENCE_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0;
        let plan = intake_layout_plan(available_height, [26.0, 55.0, 31.0]);

        assert_eq!(plan.layout, IntakeLayout::Distributed);
        assert!(plan.region_gap > theme::SPACING.section);
    }

    #[test]
    fn tall_intake_height_expands_negative_space_between_regions() {
        let reference_height = REFERENCE_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0;
        let reference = intake_layout_plan(reference_height, [26.0, 55.0, 31.0]);
        let tall = intake_layout_plan(700.0, [26.0, 55.0, 31.0]);

        assert_eq!(tall.layout, IntakeLayout::Distributed);
        assert!(tall.region_gap > reference.region_gap);
    }

    #[test]
    fn reference_intake_error_keeps_bottom_region_separated() {
        let available_height = REFERENCE_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0;
        let plan = intake_layout_plan(available_height, [26.0, 130.0, 31.0]);

        assert_eq!(plan.layout, IntakeLayout::Distributed);
        assert!(plan.region_gap >= theme::SPACING.section);
    }

    #[test]
    fn minimum_intake_height_keeps_normal_content_reachable() {
        let available_height = MINIMUM_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0;
        let plan = intake_layout_plan(available_height, [26.0, 55.0, 31.0]);

        assert_eq!(plan.layout, IntakeLayout::Distributed);
        assert!(plan.region_gap >= theme::SPACING.section);
    }

    #[test]
    fn constrained_intake_with_expanded_error_collapses_to_compact_flow() {
        let available_height = MINIMUM_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0;
        let plan = intake_layout_plan(available_height, [26.0, 150.0, 31.0]);

        assert_eq!(plan.layout, IntakeLayout::Compact);
        assert_eq!(plan.region_gap, theme::SPACING.section);
    }

    #[test]
    fn distributed_intake_places_top_middle_and_bottom_in_flow() {
        use std::cell::Cell;

        egui::__run_test_ui(|ui| {
            let available = Vec2::new(
                MAX_CONTENT_WIDTH,
                REFERENCE_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0,
            );
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let top = Cell::new(None);
            let middle = Cell::new(None);
            let bottom = Cell::new(None);
            let top_calls = Cell::new(0);
            let middle_calls = Cell::new(0);
            let bottom_calls = Cell::new(0);

            let plan = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        intake_content(
                            ui,
                            [26.0, 55.0, 31.0],
                            |ui| {
                                top_calls.set(top_calls.get() + 1);
                                let rect = ui
                                    .allocate_exact_size(
                                        Vec2::new(120.0, 26.0),
                                        egui::Sense::hover(),
                                    )
                                    .0;
                                if !ui.is_sizing_pass() {
                                    top.set(Some(rect));
                                }
                            },
                            |ui| {
                                middle_calls.set(middle_calls.get() + 1);
                                let rect = ui
                                    .allocate_exact_size(
                                        Vec2::new(160.0, 55.0),
                                        egui::Sense::hover(),
                                    )
                                    .0;
                                if !ui.is_sizing_pass() {
                                    middle.set(Some(rect));
                                }
                            },
                            |ui| {
                                bottom_calls.set(bottom_calls.get() + 1);
                                let rect = ui
                                    .allocate_exact_size(
                                        Vec2::new(180.0, 31.0),
                                        egui::Sense::hover(),
                                    )
                                    .0;
                                if !ui.is_sizing_pass() {
                                    bottom.set(Some(rect));
                                }
                            },
                        )
                    },
                )
                .inner;

            let top = top.get().expect("top region should render");
            let middle = middle.get().expect("middle region should render");
            let bottom = bottom.get().expect("bottom region should render");

            assert_eq!(top_calls.get(), 1);
            assert_eq!(middle_calls.get(), 1);
            assert_eq!(bottom_calls.get(), 1);
            assert_eq!(plan.layout, IntakeLayout::Distributed);
            assert!((top.top() - parent_rect.top()).abs() < 0.01);
            assert!((middle.center().y - parent_rect.center().y).abs() < 3.0);
            assert!((bottom.bottom() - parent_rect.bottom()).abs() < 0.01);
            assert!(middle.top() > top.bottom());
            assert!(bottom.top() > middle.bottom());
        });
    }

    #[test]
    fn compact_intake_keeps_mode_reachable_in_scroll_flow() {
        use std::cell::Cell;

        egui::__run_test_ui(|ui| {
            let available = Vec2::new(
                MINIMUM_CONTENT_SIZE.x - f32::from(CONTENT_PADDING) * 2.0,
                MINIMUM_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0,
            );
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let top = Cell::new(None);
            let middle = Cell::new(None);
            let bottom = Cell::new(None);
            let top_calls = Cell::new(0);
            let middle_calls = Cell::new(0);
            let bottom_calls = Cell::new(0);

            let plan = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::Center)),
                    |ui| {
                        intake_content(
                            ui,
                            [26.0, 150.0, 31.0],
                            |ui| {
                                top_calls.set(top_calls.get() + 1);
                                let rect = ui
                                    .allocate_exact_size(
                                        Vec2::new(120.0, 26.0),
                                        egui::Sense::hover(),
                                    )
                                    .0;
                                if !ui.is_sizing_pass() {
                                    top.set(Some(rect));
                                }
                            },
                            |ui| {
                                middle_calls.set(middle_calls.get() + 1);
                                let rect = ui
                                    .allocate_exact_size(
                                        Vec2::new(240.0, 150.0),
                                        egui::Sense::hover(),
                                    )
                                    .0;
                                if !ui.is_sizing_pass() {
                                    middle.set(Some(rect));
                                }
                            },
                            |ui| {
                                bottom_calls.set(bottom_calls.get() + 1);
                                let rect = ui
                                    .allocate_exact_size(
                                        Vec2::new(180.0, 31.0),
                                        egui::Sense::hover(),
                                    )
                                    .0;
                                if !ui.is_sizing_pass() {
                                    bottom.set(Some(rect));
                                }
                            },
                        )
                    },
                )
                .inner;

            let top = top.get().expect("top region should render");
            let middle = middle.get().expect("middle region should render");
            let bottom = bottom.get().expect("bottom region should render");

            assert_eq!(top_calls.get(), 1);
            assert_eq!(middle_calls.get(), 1);
            assert_eq!(bottom_calls.get(), 1);
            assert_eq!(plan.layout, IntakeLayout::Compact);
            assert!((top.center().x - parent_rect.center().x).abs() < 0.01);
            assert!((middle.center().x - parent_rect.center().x).abs() < 0.01);
            assert!((bottom.center().x - parent_rect.center().x).abs() < 0.01);
            assert!(middle.top() > top.bottom());
            assert!(bottom.top() > middle.bottom());
            assert!(bottom.bottom() > parent_rect.bottom());
        });
    }

    #[test]
    fn narrow_width_stacks_actions() {
        assert_eq!(action_layout(180.0, &[96.0, 84.0]), ActionLayout::Stacked);
    }

    #[test]
    fn wider_width_keeps_actions_horizontal() {
        assert_eq!(
            action_layout(191.0, &[96.0, 84.0]),
            ActionLayout::Horizontal
        );
    }

    #[test]
    fn reference_task_width_keeps_field_pair_in_columns() {
        assert_eq!(
            field_pair_layout(MAX_CONTENT_WIDTH),
            FieldPairLayout::Columns
        );
    }

    #[test]
    fn narrow_task_width_stacks_field_pair() {
        let narrow_width = MINIMUM_CONTENT_SIZE.x - f32::from(CONTENT_PADDING) * 2.0;

        assert_eq!(narrow_width, 344.0);
        assert_eq!(field_pair_layout(narrow_width), FieldPairLayout::Stacked);
    }

    #[test]
    fn horizontal_action_group_uses_intrinsic_width_gap_and_height() {
        let action_sizes = [Vec2::new(80.0, 31.0), Vec2::new(70.0, 31.0)];

        assert_eq!(
            action_group_size(
                &action_sizes,
                ActionLayout::Horizontal,
                theme::SPACING.compact,
            ),
            Vec2::new(157.0, 31.0)
        );
    }

    #[test]
    fn horizontal_action_group_is_centered_without_consuming_remaining_height() {
        egui::__run_test_ui(|ui| {
            let action_sizes = [Vec2::new(80.0, 31.0), Vec2::new(70.0, 31.0)];
            let parent_width = 320.0;
            let (parent_rect, _) =
                ui.allocate_exact_size(Vec2::new(parent_width, 140.0), egui::Sense::hover());

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        centered_action_group(
                            ui,
                            &action_sizes,
                            ActionLayout::Horizontal,
                            theme::SPACING.compact,
                            |ui| {
                                let first =
                                    ui.allocate_exact_size(action_sizes[0], egui::Sense::hover());
                                let second =
                                    ui.allocate_exact_size(action_sizes[1], egui::Sense::hover());
                                (first.0, second.0)
                            },
                        )
                    },
                )
                .inner;

            let action_bounds = response.response.rect;
            let (first, second) = response.inner;
            let left_space = first.left() - parent_rect.left();
            let right_space = parent_rect.right() - second.right();
            assert!((left_space - right_space).abs() < 0.01);
            assert_eq!(second.left() - first.right(), theme::SPACING.compact);
            assert_eq!(action_bounds.height(), theme::PRIMARY_CONTROL_HEIGHT);
            assert_eq!(action_bounds.width(), 157.0);
        });
    }

    #[test]
    fn stacked_action_group_centers_each_action_and_keeps_intrinsic_height() {
        egui::__run_test_ui(|ui| {
            let action_sizes = [Vec2::new(80.0, 31.0), Vec2::new(70.0, 31.0)];
            let parent_width = 150.0;
            let (parent_rect, _) =
                ui.allocate_exact_size(Vec2::new(parent_width, 140.0), egui::Sense::hover());

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        centered_action_group(
                            ui,
                            &action_sizes,
                            ActionLayout::Stacked,
                            theme::SPACING.compact,
                            |ui| {
                                let first =
                                    ui.allocate_exact_size(action_sizes[0], egui::Sense::hover());
                                let second =
                                    ui.allocate_exact_size(action_sizes[1], egui::Sense::hover());
                                (first.0, second.0)
                            },
                        )
                    },
                )
                .inner;

            let action_bounds = response.response.rect;
            let (first, second) = response.inner;

            assert!((first.center().x - parent_rect.center().x).abs() < 0.01);
            assert!((second.center().x - parent_rect.center().x).abs() < 0.01);
            assert_eq!(second.top() - first.bottom(), theme::SPACING.compact);
            assert_eq!(
                action_bounds.height(),
                theme::PRIMARY_CONTROL_HEIGHT * 2.0 + theme::SPACING.compact
            );
        });
    }

    #[test]
    fn focal_measurement_pass_is_invisible() {
        use std::cell::Cell;

        egui::__run_test_ui(|ui| {
            let sizing_pass_seen = Cell::new(false);
            let available = Vec2::new(470.0, 260.0);
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());

            let _ = ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(parent_rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
                |ui| {
                    focus_content(ui, |ui| {
                        if ui.is_sizing_pass() {
                            sizing_pass_seen.set(true);
                            assert!(!ui.is_visible());
                        }
                        ui.allocate_exact_size(Vec2::new(220.0, 195.0), egui::Sense::hover())
                            .0
                    })
                },
            );

            assert!(sizing_pass_seen.get());
        });
    }

    #[test]
    fn reference_height_running_focus_remains_centered_without_scrolling() {
        egui::__run_test_ui(|ui| {
            let available = Vec2::new(470.0, 323.0);
            let content_height = 168.0;
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        focus_content(ui, |ui| {
                            ui.allocate_exact_size(
                                Vec2::new(220.0, content_height),
                                egui::Sense::hover(),
                            )
                            .0
                        })
                    },
                )
                .inner;

            assert!(!response.is_scrollable());
            assert_eq!(response.viewport_size.y, available.y);
            assert!((response.inner.center().y - parent_rect.center().y).abs() < 0.5);
        });
    }

    #[test]
    fn reference_height_terminal_focus_remains_centered_without_scrolling() {
        egui::__run_test_ui(|ui| {
            let available = Vec2::new(470.0, 323.0);
            let content_height = 196.0;
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        focus_content(ui, |ui| {
                            ui.allocate_exact_size(
                                Vec2::new(220.0, content_height),
                                egui::Sense::hover(),
                            )
                            .0
                        })
                    },
                )
                .inner;

            assert!(!response.is_scrollable());
            assert_eq!(response.viewport_size.y, available.y);
            assert!((response.inner.center().y - parent_rect.center().y).abs() < 0.5);
        });
    }

    #[test]
    fn constrained_focus_content_scrolls_when_current_composition_exceeds_viewport() {
        egui::__run_test_ui(|ui| {
            let available = Vec2::new(470.0, 200.0);
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        focus_content(ui, |ui| {
                            ui.allocate_exact_size(
                                Vec2::new(220.0, FOCUS_CONTENT_MIN_HEIGHT),
                                egui::Sense::hover(),
                            )
                            .0
                        })
                    },
                )
                .inner;

            assert!(response.is_scrollable());
            assert!(response.content_size.y > response.viewport_size.y);
        });
    }

    #[test]
    fn focal_overflow_uses_rendered_height_not_230_px_reference_threshold() {
        egui::__run_test_ui(|ui| {
            let available = Vec2::new(470.0, 260.0);
            let expanded_height = 300.0;
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        focus_content(ui, |ui| {
                            ui.allocate_exact_size(
                                Vec2::new(220.0, expanded_height),
                                egui::Sense::hover(),
                            )
                            .0
                        })
                    },
                )
                .inner;

            assert!(available.y > FOCUS_CONTENT_MIN_HEIGHT);
            assert!(response.is_scrollable());
            assert!(response.content_size.y >= expanded_height);
        });
    }

    #[test]
    fn collapsing_focal_content_restores_centered_non_overflow_flow() {
        egui::__run_test_ui(|ui| {
            let available = Vec2::new(470.0, 260.0);
            let compact_height = 180.0;
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        focus_content(ui, |ui| {
                            ui.allocate_exact_size(
                                Vec2::new(220.0, compact_height),
                                egui::Sense::hover(),
                            )
                            .0
                        })
                    },
                )
                .inner;

            assert!(available.y > FOCUS_CONTENT_MIN_HEIGHT);
            assert!(!response.is_scrollable());
            assert!((response.inner.center().y - parent_rect.center().y).abs() < 0.5);
        });
    }

    #[test]
    fn bounded_content_width_never_exceeds_available_or_maximum() {
        assert_eq!(bounded_content_width(320.0), 320.0);
        assert_eq!(bounded_content_width(900.0), MAX_CONTENT_WIDTH);
    }

    #[test]
    fn output_row_keeps_fixed_controls_inline_when_filename_has_room() {
        let normal = output_row_plan(564.0, Some(68.0), 72.0);
        let wide = output_row_plan(900.0, Some(68.0), 72.0);

        assert_eq!(normal.layout, OutputRowLayout::Inline);
        assert_eq!(normal.filename_width, 410.0);
        assert_eq!(normal.flow_height(), theme::COMPACT_CONTROL_HEIGHT);
        assert_eq!(wide.layout, OutputRowLayout::Inline);
        assert!(wide.filename_width > normal.filename_width);
    }

    #[test]
    fn output_row_stacks_before_filename_or_fixed_controls_are_squeezed() {
        let mix = output_row_plan(320.0, Some(68.0), 72.0);
        let upscale = output_row_plan(240.0, None, 72.0);

        assert_eq!(mix.layout, OutputRowLayout::Stacked);
        assert_eq!(mix.filename_width, 320.0);
        assert_eq!(
            mix.flow_height(),
            theme::COMPACT_CONTROL_HEIGHT * 3.0 + theme::SPACING.compact * 2.0
        );
        assert_eq!(upscale.layout, OutputRowLayout::Stacked);
        assert_eq!(
            upscale.flow_height(),
            theme::COMPACT_CONTROL_HEIGHT * 2.0 + theme::SPACING.compact
        );
    }

    #[test]
    fn output_row_plan_sanitizes_invalid_dimensions() {
        let plan = output_row_plan(f32::NAN, Some(f32::INFINITY), f32::NEG_INFINITY);

        assert_eq!(plan.layout, OutputRowLayout::Stacked);
        assert_eq!(plan.filename_width, 0.0);
        assert_eq!(plan.leading_width, Some(f32::MAX));
    }

    #[test]
    fn audio_track_gain_stays_inline_when_identity_has_useful_width() {
        let plan = track_row_plan(564.0, theme::TRACK_GAIN_CONTROL_WIDTH);

        assert_eq!(plan.layout, TrackRowLayout::Inline);
        assert_eq!(plan.row_height, theme::SELECTABLE_ROW_MIN_HEIGHT);
        assert_eq!(TRACK_TEXT_MIN_INLINE_WIDTH, 240.0);
    }

    #[test]
    fn narrow_audio_track_moves_gain_below_identity_and_accounts_for_height() {
        let narrow_width = MINIMUM_CONTENT_SIZE.x - f32::from(CONTENT_PADDING) * 2.0;
        let plan = track_row_plan(narrow_width, theme::TRACK_GAIN_CONTROL_WIDTH);

        assert_eq!(narrow_width, 344.0);
        assert_eq!(plan.layout, TrackRowLayout::Stacked);
        assert_eq!(
            plan.row_height,
            theme::SELECTABLE_ROW_MIN_HEIGHT
                + theme::SPACING.compact
                + theme::COMPACT_CONTROL_HEIGHT
        );
    }

    #[test]
    fn operation_surface_fade_changes_opacity_without_changing_geometry() {
        let context = egui::Context::default();
        theme::install(&context);
        let mut first_rect = egui::Rect::NOTHING;
        let mut switched_rect = egui::Rect::NOTHING;

        let mut first = context.run_ui(
            egui::RawInput {
                time: Some(0.0),
                ..Default::default()
            },
            |ui| {
                surface_fade(ui, egui::Id::new("audio-mix"), |ui| {
                    first_rect = ui.label("Operation content").rect;
                });
            },
        );
        first.textures_delta.clear();

        let mut switched = context.run_ui(
            egui::RawInput {
                time: Some(0.2),
                ..Default::default()
            },
            |ui| {
                surface_fade(ui, egui::Id::new("remux"), |ui| {
                    switched_rect = ui.label("Operation content").rect;
                });
            },
        );
        switched.textures_delta.clear();

        assert_eq!(first_rect, switched_rect);
        assert_eq!(
            switched.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            std::time::Duration::ZERO
        );
    }

    #[test]
    fn settled_or_disabled_surface_fade_does_not_keep_requesting_repaint() {
        let context = egui::Context::default();
        theme::install(&context);
        let key = egui::Id::new("audio-mix");

        for time in [0.0, 0.2] {
            let mut output = context.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    surface_fade(ui, key, |ui| ui.label("Operation content"));
                },
            );
            output.textures_delta.clear();
        }
        let mut settled = context.run_ui(
            egui::RawInput {
                time: Some(0.4),
                ..Default::default()
            },
            |ui| {
                surface_fade(ui, key, |ui| ui.label("Operation content"));
            },
        );
        settled.textures_delta.clear();
        assert_eq!(
            settled.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            std::time::Duration::MAX
        );

        theme::set_motion_durations(&context, theme::MotionDurations::DISABLED);
        let mut disabled = context.run_ui(
            egui::RawInput {
                time: Some(0.6),
                ..Default::default()
            },
            |ui| {
                surface_fade(ui, egui::Id::new("remux"), |ui| {
                    ui.label("Operation content")
                });
            },
        );
        disabled.textures_delta.clear();
        assert_eq!(
            disabled.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            std::time::Duration::MAX
        );
    }

    #[test]
    fn operation_surface_anchors_action_and_keeps_content_above_it_at_all_heights() {
        egui::__run_test_ui(|ui| {
            let mut previous_content_height = 0.0;
            for height in [204.0, 323.0, 700.0] {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(620.0, height), egui::Sense::hover());
                let response = ui
                    .scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::LEFT)),
                        |ui| {
                            operation_surface(
                                ui,
                                |ui| ui.add_space(1_000.0),
                                |ui| {
                                    ui.allocate_exact_size(
                                        Vec2::new(90.0, theme::PRIMARY_CONTROL_HEIGHT),
                                        egui::Sense::click(),
                                    )
                                    .0
                                },
                            )
                        },
                    )
                    .inner;

                assert_eq!(response.content_viewport.left(), rect.left());
                assert_eq!(response.content_viewport.right(), rect.right());
                assert!(response.content_viewport.height() > previous_content_height);
                previous_content_height = response.content_viewport.height();
                assert!(response.content_viewport.bottom() <= response.action_bar.top());
                assert!((response.action_bar.bottom() - rect.bottom()).abs() <= 0.5);
                assert_eq!(
                    response.action_bar.height(),
                    theme::PRIMARY_CONTROL_HEIGHT
                        + f32::from(ACTION_BAR_VERTICAL_PADDING) * 2.0
                        + theme::SEPARATOR_WIDTH
                );
            }
        });
    }

    #[test]
    fn operation_surface_keeps_padding_inside_edge_owned_scroll_viewport() {
        egui::__run_test_ui(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(620.0, 323.0), egui::Sense::hover());
            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| operation_surface(ui, |ui| ui.add_space(900.0), |ui| ui.label("Run")),
                )
                .inner;

            assert_eq!(response.content_viewport.left(), rect.left());
            assert_eq!(response.content_viewport.right(), rect.right());
            assert!(
                response.content_gutter.left()
                    >= response.content_viewport.left() + f32::from(CONTENT_PADDING)
            );
            assert!(response.content_gutter.right() < response.content_viewport.right());
        });
    }

    #[test]
    fn deterministic_operation_surface_renders_content_and_action_once() {
        use std::cell::Cell;

        egui::__run_test_ui(|ui| {
            let content_calls = Cell::new(0);
            let action_calls = Cell::new(0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(620.0, 323.0), egui::Sense::hover());

            let _ = ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
                |ui| {
                    operation_surface(
                        ui,
                        |ui| {
                            content_calls.set(content_calls.get() + 1);
                            ui.add_space(900.0)
                        },
                        |ui| {
                            action_calls.set(action_calls.get() + 1);
                            ui.label("Run")
                        },
                    )
                },
            );

            assert_eq!(content_calls.get(), 1);
            assert_eq!(action_calls.get(), 1);
        });
    }
    #[test]
    fn wide_running_and_terminal_surfaces_keep_independent_readability_bound() {
        egui::__run_test_ui(|ui| {
            let available = Vec2::new(1600.0, 260.0);
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        focus_content(ui, |ui| {
                            ui.allocate_exact_size(Vec2::new(220.0, 120.0), egui::Sense::hover())
                                .0
                        })
                    },
                )
                .inner;

            assert_eq!(response.viewport_size.x, FOCUS_CONTENT_MAX_WIDTH);
        });
    }

    #[test]
    fn filename_overflow_policy_is_single_line_ellipsis() {
        assert_eq!(PATH_TEXT_OVERFLOW, TextOverflowPolicy::Ellipsis);
    }
}

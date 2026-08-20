use egui::{
    Align, Frame, InnerResponse, Layout, Margin, Rect, ScrollArea, Style, Ui, UiBuilder, Vec2, vec2,
};

use super::theme;

/// Reference outer-window geometry selected by `.project/design/LAYOUT.md`.
pub const REFERENCE_WINDOW_SIZE: Vec2 = Vec2::new(620.0, 410.0);

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

/// Maximum width for bounded sparse content at the reference window size after content insets.
///
/// File intake keeps this reference-width bound. Ready/configuration surfaces intentionally do not.
pub const MAX_CONTENT_WIDTH: f32 = REFERENCE_WINDOW_SIZE.x - 2.0 * CONTENT_PADDING as f32;

/// Minimum comfortable width for each field in a responsive two-field row.
pub const FIELD_PAIR_MIN_COLUMN_WIDTH: f32 = 180.0;

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
pub enum IntakeLayout {
    Distributed,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntakeLayoutPlan {
    pub layout: IntakeLayout,
    pub region_gap: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListBehavior {
    Flow,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListViewport {
    pub behavior: ListBehavior,
    pub max_height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskOverflow {
    Flow,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaskSurfacePlan {
    pub overflow: TaskOverflow,
    pub list: ListViewport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverflowPolicy {
    Ellipsis,
}

/// Apply the selected Quiet Focus content inset while preserving the themed panel fill.
pub fn content_frame(style: &Style) -> Frame {
    Frame::central_panel(style).inner_margin(Margin::same(CONTENT_PADDING))
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
/// The regions are measured at the current responsive content width. Remaining vertical space is
/// distributed evenly between them, keeping the title near the top, the intake action around the
/// visual middle, and the mode selector at the bottom. When the shared section gaps no longer fit,
/// the surface collapses to compact top-down flow inside a vertical scroll area.
pub fn intake_content(
    ui: &mut Ui,
    mut show_top: impl FnMut(&mut Ui),
    mut show_middle: impl FnMut(&mut Ui),
    mut show_bottom: impl FnMut(&mut Ui),
) -> IntakeLayoutPlan {
    let available = safe_available_size(ui.available_size());
    let content_width = bounded_content_width(available.x);
    let measure_rect = Rect::from_min_size(ui.cursor().min, vec2(content_width, available.y));

    let region_heights = [
        measure_intake_region(ui, "intake_top_measure", measure_rect, &mut show_top),
        measure_intake_region(ui, "intake_middle_measure", measure_rect, &mut show_middle),
        measure_intake_region(ui, "intake_bottom_measure", measure_rect, &mut show_bottom),
    ];
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

fn measure_intake_region(
    ui: &mut Ui,
    id_salt: &'static str,
    max_rect: Rect,
    add_contents: &mut impl FnMut(&mut Ui),
) -> f32 {
    let mut measure_ui = ui.new_child(
        UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(max_rect)
            .layout(Layout::top_down(Align::Center))
            .sizing_pass()
            .invisible(),
    );
    measure_ui.spacing_mut().item_spacing.y = 0.0;
    add_contents(&mut measure_ui);
    safe_dimension(measure_ui.min_rect().height())
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
/// copy and expanded bounded diagnostics. If that measured height fits, the real surface keeps
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

/// Render a fluid ready/configuration surface with vertical overflow only when required.
///
/// The surface consumes the current padded viewport width. Constrained heights use an egui
/// [`ScrollArea`] so required actions remain reachable without changing control metrics or the
/// application's minimum window size.
pub fn task_surface<R>(
    ui: &mut Ui,
    overflow: TaskOverflow,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let available = safe_available_size(ui.available_size());
    let content_width = available.x;

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.allocate_ui_with_layout(
            vec2(content_width, available.y),
            Layout::top_down(Align::LEFT),
            |ui| match overflow {
                TaskOverflow::Flow => add_contents(ui),
                TaskOverflow::Scroll => {
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, add_contents)
                        .inner
                }
            },
        )
    })
    .inner
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

/// Width left for a flexible row element after a fixed-width sibling and their shared gap.
///
/// Composite controls use this rather than owning their own responsive-width policy.
pub fn flexible_row_width(available_width: f32, fixed_width: f32, gap: f32) -> f32 {
    (safe_dimension(available_width) - safe_dimension(fixed_width) - safe_dimension(gap)).max(0.0)
}

/// Reserve flow space after a potentially long list and request scrolling only when needed.
pub fn list_viewport(
    content_height: f32,
    available_height: f32,
    reserved_flow_height: f32,
) -> ListViewport {
    let content_height = safe_dimension(content_height);
    let available_height = safe_dimension(available_height);
    let reserved_flow_height = safe_dimension(reserved_flow_height);
    let remaining_height = (available_height - reserved_flow_height).max(0.0);
    let max_height = content_height.min(remaining_height);
    let behavior = if content_height > remaining_height {
        ListBehavior::Scroll
    } else {
        ListBehavior::Flow
    };

    ListViewport {
        behavior,
        max_height,
    }
}

/// Plan nested list and task-surface overflow without sacrificing control hit targets.
///
/// Lists may consume all vertical space left after the fixed controls. Nested scrolling is used
/// only when list content exceeds that remainder. If even one usable option row would no longer
/// fit, the task surface becomes vertically scrollable and the nested list retains a one-row
/// viewport so the primary action stays reachable through outer scrolling.
pub fn task_surface_plan(
    available_height: f32,
    fixed_flow_height: f32,
    list_content_height: f32,
) -> TaskSurfacePlan {
    let available_height = safe_dimension(available_height);
    let fixed_flow_height = safe_dimension(fixed_flow_height);
    let list_content_height = safe_dimension(list_content_height);
    let minimum_list_height = theme::SELECTABLE_ROW_MIN_HEIGHT
        .max(theme::UPSCALE_TARGET_ROW_MIN_HEIGHT)
        .min(list_content_height);
    let list = list_viewport(list_content_height, available_height, fixed_flow_height);

    if fixed_flow_height > available_height
        || (list_content_height > 0.0 && list.max_height < minimum_list_height)
    {
        TaskSurfacePlan {
            overflow: TaskOverflow::Scroll,
            list: ListViewport {
                behavior: if list_content_height > minimum_list_height {
                    ListBehavior::Scroll
                } else {
                    ListBehavior::Flow
                },
                max_height: minimum_list_height,
            },
        }
    } else {
        TaskSurfacePlan {
            overflow: TaskOverflow::Flow,
            list,
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
    use egui::{Style, Vec2};

    use super::{
        ActionLayout, CONTENT_PADDING, DIAGNOSTIC_MAX_HEIGHT, FIELD_PAIR_MIN_COLUMN_WIDTH,
        FOCUS_CONTENT_MAX_WIDTH, FOCUS_CONTENT_MIN_HEIGHT, FieldPairLayout, IntakeLayout,
        ListBehavior, MAX_CONTENT_WIDTH, MINIMUM_CONTENT_SIZE, PATH_TEXT_OVERFLOW,
        PROGRESS_TRACK_HEIGHT, REFERENCE_CONTENT_SIZE, REFERENCE_WINDOW_SIZE, TaskOverflow,
        TextOverflowPolicy, WINDOW_CHROME_HEIGHT, action_group_size, action_layout,
        bounded_content_width, centered_action_group, content_frame, field_pair_layout,
        flexible_row_width, focus_content, intake_content, intake_layout_plan, list_viewport,
        task_surface, task_surface_plan,
    };
    use crate::ui::theme;

    #[test]
    fn quiet_focus_reference_geometry_is_centralized() {
        assert_eq!(REFERENCE_WINDOW_SIZE, Vec2::new(620.0, 410.0));
        assert_eq!(WINDOW_CHROME_HEIGHT, 31.0);
        assert_eq!(REFERENCE_CONTENT_SIZE, Vec2::new(620.0, 379.0));
        assert_eq!(MINIMUM_CONTENT_SIZE, Vec2::new(400.0, 260.0));
        assert_eq!(CONTENT_PADDING, 28);
        assert_eq!(MAX_CONTENT_WIDTH, 564.0);
        assert_eq!(FIELD_PAIR_MIN_COLUMN_WIDTH, 180.0);
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
    fn content_frame_uses_selected_quiet_focus_padding() {
        let frame = content_frame(&Style::default());

        assert_eq!(frame.inner_margin.left, CONTENT_PADDING);
        assert_eq!(frame.inner_margin.right, CONTENT_PADDING);
        assert_eq!(frame.inner_margin.top, CONTENT_PADDING);
        assert_eq!(frame.inner_margin.bottom, CONTENT_PADDING);
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

            let plan = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        intake_content(
                            ui,
                            |ui| {
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

            let plan = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::Center)),
                    |ui| {
                        intake_content(
                            ui,
                            |ui| {
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
    fn flexible_row_width_reserves_fixed_controls_without_going_negative() {
        assert_eq!(flexible_row_width(320.0, 72.0, 7.0), 241.0);
        assert_eq!(flexible_row_width(40.0, 72.0, 7.0), 0.0);
        assert_eq!(flexible_row_width(f32::NAN, 72.0, 7.0), 0.0);
    }

    #[test]
    fn long_list_chooses_scrolling_and_reserves_following_actions() {
        let viewport = list_viewport(500.0, 320.0, 80.0);

        assert_eq!(viewport.behavior, ListBehavior::Scroll);
        assert_eq!(viewport.max_height, 240.0);
    }

    #[test]
    fn short_list_remains_in_normal_flow() {
        let viewport = list_viewport(180.0, 320.0, 80.0);

        assert_eq!(viewport.behavior, ListBehavior::Flow);
        assert_eq!(viewport.max_height, 180.0);
    }

    #[test]
    fn minimum_drawable_height_leaves_204_px_after_quiet_focus_padding() {
        let usable_height = MINIMUM_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0;

        assert_eq!(MINIMUM_CONTENT_SIZE.y, 260.0);
        assert_eq!(f32::from(CONTENT_PADDING), 28.0);
        assert_eq!(usable_height, 204.0);
    }

    #[test]
    fn task_list_uses_all_remaining_height_before_outer_overflow_is_needed() {
        let plan = task_surface_plan(280.0, 200.0, 108.0);

        assert_eq!(plan.overflow, TaskOverflow::Flow);
        assert_eq!(plan.list.behavior, ListBehavior::Scroll);
        assert_eq!(plan.list.max_height, 80.0);
    }

    #[test]
    fn fixed_controls_still_use_outer_scroll_when_the_option_list_is_empty() {
        let plan = task_surface_plan(204.0, 220.0, 0.0);

        assert_eq!(plan.overflow, TaskOverflow::Scroll);
        assert_eq!(plan.list.behavior, ListBehavior::Flow);
        assert_eq!(plan.list.max_height, 0.0);
    }

    #[test]
    fn constrained_task_surface_scrolls_before_list_becomes_unusable() {
        let plan = task_surface_plan(204.0, 205.0, 108.0);

        assert_eq!(plan.overflow, TaskOverflow::Scroll);
        assert_eq!(plan.list.behavior, ListBehavior::Scroll);
        assert_eq!(plan.list.max_height, theme::UPSCALE_TARGET_ROW_MIN_HEIGHT);
    }

    #[test]
    fn reference_height_uses_all_space_left_for_a_long_list() {
        let usable_reference_height = REFERENCE_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0;
        let plan = task_surface_plan(usable_reference_height, 205.0, 500.0);

        assert_eq!(usable_reference_height, 323.0);
        assert_eq!(plan.overflow, TaskOverflow::Flow);
        assert_eq!(plan.list.behavior, ListBehavior::Scroll);
        assert_eq!(plan.list.max_height, 118.0);
    }

    #[test]
    fn taller_task_surface_increases_list_viewport_until_content_fits() {
        let reference = task_surface_plan(323.0, 205.0, 500.0);
        let tall = task_surface_plan(600.0, 205.0, 500.0);
        let taller_than_content = task_surface_plan(800.0, 205.0, 500.0);

        assert_eq!(reference.list.max_height, 118.0);
        assert_eq!(tall.list.max_height, 395.0);
        assert!(tall.list.max_height > reference.list.max_height);
        assert_eq!(tall.list.behavior, ListBehavior::Scroll);
        assert_eq!(taller_than_content.list.max_height, 500.0);
        assert_eq!(taller_than_content.list.behavior, ListBehavior::Flow);
    }

    #[test]
    fn three_upscale_targets_flow_at_reference_height_when_fixed_controls_leave_room() {
        let usable_reference_height = REFERENCE_CONTENT_SIZE.y - f32::from(CONTENT_PADDING) * 2.0;
        let content_height = theme::UPSCALE_TARGET_ROW_MIN_HEIGHT * 3.0;
        let fixed_flow_height = usable_reference_height - content_height;

        let plan = task_surface_plan(usable_reference_height, fixed_flow_height, content_height);

        assert_eq!(plan.overflow, TaskOverflow::Flow);
        assert_eq!(plan.list.behavior, ListBehavior::Flow);
        assert_eq!(plan.list.max_height, content_height);
    }

    #[test]
    fn tall_audio_surface_shows_more_tracks_before_nested_scrolling() {
        let content_height = theme::SELECTABLE_ROW_MIN_HEIGHT * 20.0;
        let reference = task_surface_plan(323.0, 205.0, content_height);
        let tall = task_surface_plan(700.0, 205.0, content_height);

        assert_eq!(reference.list.behavior, ListBehavior::Scroll);
        assert_eq!(tall.list.behavior, ListBehavior::Scroll);
        assert!(tall.list.max_height > reference.list.max_height);
        assert_eq!(reference.overflow, TaskOverflow::Flow);
        assert_eq!(tall.overflow, TaskOverflow::Flow);
    }

    #[test]
    fn layout_helpers_never_return_negative_or_non_finite_sizes() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -10.0] {
            let width = bounded_content_width(value);
            let viewport = list_viewport(value, value, value);

            assert!(width.is_finite());
            assert!(width >= 0.0);
            assert!(viewport.max_height.is_finite());
            assert!(viewport.max_height >= 0.0);
        }
    }

    #[test]
    fn task_surface_width_grows_with_required_viewports_after_shared_gutters() {
        egui::__run_test_ui(|ui| {
            let mut previous = 0.0;

            for viewport_width in [400.0, 620.0, 900.0, 1200.0, 1600.0] {
                let available_width = viewport_width - f32::from(CONTENT_PADDING) * 2.0;
                let (parent_rect, _) =
                    ui.allocate_exact_size(Vec2::new(available_width, 80.0), egui::Sense::hover());

                let rendered_width = ui
                    .scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(parent_rect)
                            .layout(egui::Layout::top_down(egui::Align::LEFT)),
                        |ui| task_surface(ui, TaskOverflow::Flow, |ui| ui.available_width()).inner,
                    )
                    .inner;

                assert!((rendered_width - available_width).abs() < 0.01);
                assert!(rendered_width > previous);
                previous = rendered_width;
            }
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

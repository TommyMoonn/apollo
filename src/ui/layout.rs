use egui::{Align, Frame, InnerResponse, Layout, Margin, Style, Ui, Vec2, vec2};

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

/// Maximum readable task width at the reference window size after content insets.
pub const MAX_CONTENT_WIDTH: f32 = REFERENCE_WINDOW_SIZE.x - 2.0 * CONTENT_PADDING as f32;

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

/// Center a feature surface while keeping its contents top-down and left aligned.
///
/// Audio Mix and Upscale screens can share this policy without owning fixed viewport positions.
pub fn task_content<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let available = safe_available_size(ui.available_size());
    let content_width = bounded_content_width(available.x);

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.allocate_ui_with_layout(
            vec2(content_width, available.y),
            Layout::top_down(Align::LEFT),
            add_contents,
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

/// Bound readable task content to the Quiet Focus reference width without returning invalid sizes.
pub fn bounded_content_width(available_width: f32) -> f32 {
    safe_dimension(available_width).min(MAX_CONTENT_WIDTH)
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
    let max_height = (available_height - reserved_flow_height).max(0.0);
    let behavior = if content_height > max_height {
        ListBehavior::Scroll
    } else {
        ListBehavior::Flow
    };

    ListViewport {
        behavior,
        max_height,
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
        ActionLayout, CONTENT_PADDING, ListBehavior, MAX_CONTENT_WIDTH, MINIMUM_CONTENT_SIZE,
        PATH_TEXT_OVERFLOW, PROGRESS_TRACK_HEIGHT, REFERENCE_CONTENT_SIZE, REFERENCE_WINDOW_SIZE,
        TextOverflowPolicy, WINDOW_CHROME_HEIGHT, action_layout, bounded_content_width,
        content_frame, list_viewport,
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
        assert_eq!(theme::PRIMARY_CONTROL_HEIGHT, 31.0);
        assert_eq!(theme::COMPACT_CONTROL_HEIGHT, 30.0);
        assert_eq!(theme::SPACING.compact, 7.0);
        assert_eq!(theme::SPACING.related, 11.0);
        assert_eq!(theme::SPACING.section, 24.0);
        assert_eq!(theme::SPACING.spacious, 40.0);
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
    fn bounded_content_width_never_exceeds_available_or_maximum() {
        assert_eq!(bounded_content_width(320.0), 320.0);
        assert_eq!(bounded_content_width(900.0), MAX_CONTENT_WIDTH);
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
        assert_eq!(viewport.max_height, 240.0);
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
    fn filename_overflow_policy_is_single_line_ellipsis() {
        assert_eq!(PATH_TEXT_OVERFLOW, TextOverflowPolicy::Ellipsis);
    }
}

use egui::{
    Align, Align2, Color32, CornerRadius, InnerResponse, Label, Layout, Popup, Rect, Response,
    RichText, Sense, Stroke, StrokeKind, TextStyle, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType,
    pos2, vec2,
};

use super::{fonts, layout, theme};

const BUTTON_HORIZONTAL_PADDING: f32 = theme::SPACING.related;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionIndicator {
    Checkbox,
    Radio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineStatusKind {
    Status,
    Error,
}

pub struct OutputPathRowResponse {
    pub path: Response,
    pub browse: Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ButtonKind {
    tone: theme::ControlTone,
    selected: bool,
    compact: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectableRowVisuals {
    background: Color32,
    indicator_fill: Color32,
    indicator_foreground: Color32,
    indicator_border: Color32,
}

#[must_use]
pub fn primary_button(ui: &mut Ui, label: &str) -> Response {
    primary_button_enabled(ui, label, true)
}

#[must_use]
pub fn primary_button_enabled(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(
        ui,
        label,
        enabled,
        ButtonKind {
            tone: theme::ControlTone::Primary,
            selected: false,
            compact: false,
        },
    )
}

#[must_use]
pub fn secondary_button(ui: &mut Ui, label: &str) -> Response {
    button(
        ui,
        label,
        true,
        ButtonKind {
            tone: theme::ControlTone::Standard,
            selected: false,
            compact: false,
        },
    )
}

#[must_use]
pub fn destructive_button(ui: &mut Ui, label: &str) -> Response {
    button(
        ui,
        label,
        true,
        ButtonKind {
            tone: theme::ControlTone::Destructive,
            selected: false,
            compact: false,
        },
    )
}

/// A compact non-pill mode choice. The parent layout decides whether mode choices are inline or
/// stacked; this primitive owns only intrinsic control sizing and selected-state painting.
#[must_use]
pub fn mode_button(ui: &mut Ui, label: &str, selected: bool) -> Response {
    button_with_horizontal_padding(
        ui,
        label,
        true,
        ButtonKind {
            tone: theme::ControlTone::Standard,
            selected,
            compact: false,
        },
        theme::MODE_BUTTON_HORIZONTAL_PADDING,
    )
}

/// Intrinsic size of a standard-height mode/tool control. Parent layouts use this to choose
/// responsive horizontal versus stacked composition without duplicating control metrics.
pub fn mode_button_intrinsic_size(ui: &Ui, label: &str) -> Vec2 {
    button_intrinsic_size(ui, label, false, theme::MODE_BUTTON_HORIZONTAL_PADDING)
}

/// Compact selector trigger used by simple container/codec dropdowns.
#[must_use]
pub fn select_button(ui: &mut Ui, label: &str) -> Response {
    button(
        ui,
        &format!("{label}  ⌄"),
        true,
        ButtonKind {
            tone: theme::ControlTone::Standard,
            selected: false,
            compact: true,
        },
    )
}

/// Compact quiet action for output rows such as Browse.
#[must_use]
pub fn compact_secondary_button(ui: &mut Ui, label: &str) -> Response {
    button(
        ui,
        label,
        true,
        ButtonKind {
            tone: theme::ControlTone::Standard,
            selected: false,
            compact: true,
        },
    )
}

/// Unframed secondary text action for low-priority operations such as Change, Select all, Clear,
/// or Show details. Keyboard focus still receives the shared focus outline.
#[must_use]
pub fn plain_button(ui: &mut Ui, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        plain_button_text_style().resolve(ui.style()),
        theme::SECONDARY,
    );
    let desired_size = vec2(
        galley.size().x,
        theme::COMPACT_CONTROL_HEIGHT.max(galley.size().y),
    );
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    let state = interaction_state(&response);
    let foreground = match state {
        theme::InteractionState::Disabled => theme::DISABLED,
        theme::InteractionState::Hovered | theme::InteractionState::Pressed => theme::FOREGROUND,
        theme::InteractionState::Idle => theme::SECONDARY,
    };

    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        plain_button_text_style().resolve(ui.style()),
        foreground,
    );
    paint_keyboard_focus(ui, &response, rect);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, response.enabled(), label));
    response
}

/// Simple dropdown shell built from the shared compact selector primitive and egui's popup owner.
/// The caller owns the option domain/state and supplies the menu contents.
pub fn select_menu<R>(
    ui: &mut Ui,
    selected_text: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let response = select_button(ui, selected_text);
    let inner = Popup::menu(&response)
        .width(response.rect.width())
        .show(add_contents)
        .map(|inner_response| inner_response.inner);

    InnerResponse { inner, response }
}

/// Flat selectable row for track and target choices.
///
/// `minimum_height` may raise the shared 34 px row floor for a specific planned row type, but the
/// control will never shrink below the central theme metric.
#[must_use]
pub fn selectable_row(
    ui: &mut Ui,
    title: &str,
    metadata: Option<&str>,
    selected: bool,
    indicator: SelectionIndicator,
    minimum_height: f32,
) -> Response {
    let row_height = finite_nonnegative(minimum_height).max(theme::SELECTABLE_ROW_MIN_HEIGHT);
    let desired_size = vec2(finite_nonnegative(ui.available_width()), row_height);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    let state = interaction_state(&response);
    let visuals = selectable_row_visuals(state, selected);

    ui.painter().rect_filled(rect, 0.0, visuals.background);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(theme::SEPARATOR_WIDTH, theme::SEPARATOR),
    );

    let indicator_rect = Rect::from_center_size(
        pos2(
            rect.left()
                + theme::SELECTABLE_ROW_HORIZONTAL_PADDING
                + theme::SELECTABLE_ROW_INDICATOR_COLUMN * 0.5,
            rect.center().y,
        ),
        Vec2::splat(theme::SELECTION_INDICATOR_SIZE),
    );
    paint_selection_indicator(ui, indicator_rect, indicator, selected, visuals);

    let text_left = rect.left()
        + theme::SELECTABLE_ROW_HORIZONTAL_PADDING
        + theme::SELECTABLE_ROW_INDICATOR_COLUMN
        + theme::SELECTABLE_ROW_CONTENT_GAP;
    let text_right = rect.right() - theme::SELECTABLE_ROW_HORIZONTAL_PADDING;
    let metadata_font = fonts::technical_text_style().resolve(ui.style());
    let metadata_width = metadata
        .map(|text| {
            ui.painter()
                .layout_no_wrap(
                    text.to_owned(),
                    metadata_font.clone(),
                    theme::SECONDARY_FAINT,
                )
                .size()
                .x
        })
        .unwrap_or(0.0);
    let metadata_gap = if metadata.is_some() {
        theme::SELECTABLE_ROW_CONTENT_GAP
    } else {
        0.0
    };
    let title_right = (text_right - metadata_width - metadata_gap).max(text_left);
    let title_rect = Rect::from_min_max(
        pos2(text_left, rect.top()),
        pos2(title_right, rect.bottom()),
    );

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(title_rect)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.add(
                Label::new(RichText::new(title).text_style(TextStyle::Body))
                    .truncate()
                    .selectable(false),
            );
        },
    );

    if let Some(metadata) = metadata {
        ui.painter().text(
            pos2(text_right, rect.center().y),
            Align2::RIGHT_CENTER,
            metadata,
            metadata_font,
            theme::SECONDARY_FAINT,
        );
    }

    paint_keyboard_focus(ui, &response, rect);
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Button, response.enabled(), selected, title)
    });
    response
}

/// Bordered technical path field. The parent supplies the width so output-row composition remains
/// with the responsive layout owner rather than this primitive.
#[must_use]
pub fn path_field(ui: &mut Ui, text: &str, width: f32) -> Response {
    let desired_size = vec2(finite_nonnegative(width), theme::COMPACT_CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

    ui.painter()
        .rect_filled(rect, theme::CONTROL_RADIUS, theme::BACKGROUND);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(theme::CONTROL_RADIUS),
        Stroke::new(theme::SEPARATOR_WIDTH, theme::SEPARATOR),
        StrokeKind::Inside,
    );

    let text_rect = rect.shrink2(vec2(theme::PATH_FIELD_HORIZONTAL_PADDING, 0.0));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(text_rect)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.add(
                Label::new(
                    RichText::new(text)
                        .text_style(fonts::technical_text_style())
                        .color(theme::SECONDARY_FAINT),
                )
                .truncate()
                .selectable(false),
            );
        },
    );

    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, response.enabled(), text));
    response
}

/// Responsive path + Browse composite used by output rows that do not have a leading selector.
/// Width allocation is delegated to the shared layout owner; this function only composes the two
/// shared primitives.
#[must_use]
pub fn output_path_row(ui: &mut Ui, path: &str, browse_label: &str) -> OutputPathRowResponse {
    let browse_width = button_intrinsic_size(ui, browse_label, true, BUTTON_HORIZONTAL_PADDING).x;
    let path_width =
        layout::flexible_row_width(ui.available_width(), browse_width, theme::SPACING.compact);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACING.compact;
        let path = path_field(ui, path, path_width);
        let browse = compact_secondary_button(ui, browse_label);
        OutputPathRowResponse { path, browse }
    })
    .inner
}

/// Thin rectangular progress rail. The fraction is display-only and deliberately carries no
/// completion semantics; authoritative process state remains outside this primitive.
#[must_use]
pub fn progress_bar(ui: &mut Ui, fraction: f32) -> Response {
    let fraction = displayed_progress_fraction(fraction);
    let desired_size = vec2(
        finite_nonnegative(ui.available_width()),
        layout::PROGRESS_TRACK_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

    ui.painter()
        .rect_filled(rect, theme::PROGRESS_RADIUS, progress_rail_color());

    let fill_width = rect.width() * fraction;
    if fill_width > 0.0 {
        let fill = Rect::from_min_max(rect.min, pos2(rect.left() + fill_width, rect.bottom()));
        ui.painter()
            .rect_filled(fill, theme::PROGRESS_RADIUS, theme::PROGRESS);
    }

    response
}

#[must_use]
pub fn inline_status(ui: &mut Ui, text: &str, kind: InlineStatusKind) -> Response {
    let color = inline_status_color(kind);

    ui.add(Label::new(RichText::new(text).text_style(TextStyle::Body).color(color)).wrap())
}

fn inline_status_color(kind: InlineStatusKind) -> Color32 {
    match kind {
        InlineStatusKind::Status => theme::SECONDARY,
        InlineStatusKind::Error => theme::FOREGROUND,
    }
}

#[must_use]
pub fn section_label(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(TextStyle::Small)
                .color(theme::SECONDARY),
        )
        .selectable(false),
    )
}

pub fn separator(ui: &mut Ui) {
    let available_width = finite_nonnegative(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(
        vec2(available_width, theme::SEPARATOR_WIDTH),
        Sense::hover(),
    );
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(theme::SEPARATOR_WIDTH, theme::SEPARATOR),
    );
}

fn button(ui: &mut Ui, label: &str, enabled: bool, kind: ButtonKind) -> Response {
    button_with_horizontal_padding(ui, label, enabled, kind, BUTTON_HORIZONTAL_PADDING)
}

fn button_with_horizontal_padding(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
    kind: ButtonKind,
    horizontal_padding: f32,
) -> Response {
    let effective_enabled = enabled && ui.is_enabled();
    ui.add_enabled_ui(effective_enabled, |ui| {
        let height = if kind.compact {
            theme::COMPACT_CONTROL_HEIGHT
        } else {
            theme::PRIMARY_CONTROL_HEIGHT
        };
        let desired_size = button_intrinsic_size(ui, label, kind.compact, horizontal_padding);
        debug_assert_eq!(desired_size.y, height);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
        let state = interaction_state(&response);
        let colors = if kind.selected {
            theme::selectable_colors(state, true)
        } else {
            theme::control_colors(kind.tone, state)
        };

        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::CONTROL_RADIUS),
            colors.background,
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme::CONTROL_RADIUS),
            Stroke::new(theme::SEPARATOR_WIDTH, colors.border),
            StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            button_text_style().resolve(ui.style()),
            colors.foreground,
        );
        paint_keyboard_focus(ui, &response, rect);

        response.widget_info(|| {
            if kind.selected {
                WidgetInfo::selected(WidgetType::Button, response.enabled(), true, label)
            } else {
                WidgetInfo::labeled(WidgetType::Button, response.enabled(), label)
            }
        });
        response
    })
    .inner
}

fn button_intrinsic_size(ui: &Ui, label: &str, compact: bool, horizontal_padding: f32) -> Vec2 {
    let height = if compact {
        theme::COMPACT_CONTROL_HEIGHT
    } else {
        theme::PRIMARY_CONTROL_HEIGHT
    };
    let font_id = button_text_style().resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font_id, theme::FOREGROUND);

    vec2(galley.size().x + horizontal_padding * 2.0, height)
}

fn button_text_style() -> TextStyle {
    TextStyle::Button
}

fn plain_button_text_style() -> TextStyle {
    button_text_style()
}

fn progress_rail_color() -> Color32 {
    theme::SEPARATOR
}

fn interaction_state(response: &Response) -> theme::InteractionState {
    if !response.enabled() {
        theme::InteractionState::Disabled
    } else if response.is_pointer_button_down_on() {
        theme::InteractionState::Pressed
    } else if response.hovered() || response.highlighted() {
        theme::InteractionState::Hovered
    } else {
        theme::InteractionState::Idle
    }
}

fn selectable_row_visuals(state: theme::InteractionState, selected: bool) -> SelectableRowVisuals {
    let interaction = theme::control_colors(theme::ControlTone::Standard, state);

    if selected && !matches!(state, theme::InteractionState::Disabled) {
        SelectableRowVisuals {
            background: interaction.background,
            indicator_fill: theme::SELECTION,
            indicator_foreground: theme::BACKGROUND,
            indicator_border: theme::SELECTION,
        }
    } else {
        let indicator_border = if matches!(state, theme::InteractionState::Disabled) {
            theme::DISABLED
        } else {
            theme::BORDER_STRONG
        };
        SelectableRowVisuals {
            background: interaction.background,
            indicator_fill: theme::BACKGROUND,
            indicator_foreground: interaction.foreground,
            indicator_border,
        }
    }
}

fn paint_selection_indicator(
    ui: &Ui,
    rect: Rect,
    indicator: SelectionIndicator,
    selected: bool,
    visuals: SelectableRowVisuals,
) {
    match indicator {
        SelectionIndicator::Checkbox => {
            ui.painter().rect_filled(rect, 0.0, visuals.indicator_fill);
            ui.painter().rect_stroke(
                rect,
                0.0,
                Stroke::new(theme::SEPARATOR_WIDTH, visuals.indicator_border),
                StrokeKind::Inside,
            );
            if selected {
                let left = pos2(rect.left() + 2.5, rect.center().y);
                let middle = pos2(rect.left() + 5.0, rect.bottom() - 3.0);
                let right = pos2(rect.right() - 2.0, rect.top() + 3.0);
                let stroke = Stroke::new(1.25, visuals.indicator_foreground);
                ui.painter().line_segment([left, middle], stroke);
                ui.painter().line_segment([middle, right], stroke);
            }
        }
        SelectionIndicator::Radio => {
            let radius = rect.width() * 0.5;
            ui.painter()
                .circle_filled(rect.center(), radius, visuals.indicator_fill);
            ui.painter().circle_stroke(
                rect.center(),
                radius,
                Stroke::new(theme::SEPARATOR_WIDTH, visuals.indicator_border),
            );
            if selected {
                ui.painter()
                    .circle_filled(rect.center(), 3.0, visuals.indicator_foreground);
            }
        }
    }
}

fn paint_keyboard_focus(ui: &Ui, response: &Response, rect: Rect) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(theme::FOCUS_OFFSET),
            CornerRadius::same(theme::CONTROL_RADIUS),
            Stroke::new(theme::FOCUS_STROKE_WIDTH, theme::FOCUS),
            StrokeKind::Outside,
        );
    }
}

fn displayed_progress_fraction(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use egui::{FontFamily, FontId, Sense, TextStyle};

    use super::{
        InlineStatusKind, SelectionIndicator, button_text_style, displayed_progress_fraction,
        inline_status_color, mode_button, mode_button_intrinsic_size, plain_button_text_style,
        primary_button_enabled, progress_bar, progress_rail_color, selectable_row,
        selectable_row_visuals,
    };
    use crate::ui::{fonts, layout, theme};

    #[test]
    fn selected_and_unselected_rows_keep_selection_separate_from_interaction() {
        let unselected = selectable_row_visuals(theme::InteractionState::Idle, false);
        let selected = selectable_row_visuals(theme::InteractionState::Idle, true);
        let selected_hover = selectable_row_visuals(theme::InteractionState::Hovered, true);

        assert_eq!(unselected.background, theme::BACKGROUND);
        assert_eq!(unselected.indicator_fill, theme::BACKGROUND);
        assert_eq!(selected.background, theme::BACKGROUND);
        assert_eq!(selected.indicator_fill, theme::SELECTION);
        assert_eq!(selected.indicator_foreground, theme::BACKGROUND);
        assert_eq!(selected_hover.background, theme::HOVER);
        assert_eq!(selected_hover.indicator_fill, theme::SELECTION);
    }

    #[test]
    fn disabled_controls_reject_interaction() {
        egui::__run_test_ui(|ui| {
            let response = primary_button_enabled(ui, "Mix audio", false);

            assert!(!response.enabled());
            assert!(!response.clicked());
        });
    }

    #[test]
    fn clickable_primitives_preserve_egui_keyboard_activation_semantics() {
        egui::__run_test_ui(|ui| {
            install_test_technical_style(ui);
            let button = mode_button(ui, "Audio Mix", false);
            let row = selectable_row(
                ui,
                "Desktop Audio",
                Some("Stereo"),
                false,
                SelectionIndicator::Checkbox,
                34.0,
            );

            assert_eq!(button.sense, Sense::click());
            assert_eq!(row.sense, Sense::click());
        });
    }

    #[test]
    fn mode_controls_use_quiet_focus_intrinsic_padding() {
        egui::__run_test_ui(|ui| {
            let size = mode_button_intrinsic_size(ui, "Audio Mix");
            let galley = ui.painter().layout_no_wrap(
                "Audio Mix".to_owned(),
                button_text_style().resolve(ui.style()),
                theme::FOREGROUND,
            );

            assert_eq!(size.y, theme::PRIMARY_CONTROL_HEIGHT);
            assert_eq!(
                size.x,
                galley.size().x + theme::MODE_BUTTON_HORIZONTAL_PADDING * 2.0
            );
        });
    }

    #[test]
    fn plain_text_actions_use_control_typography() {
        assert_eq!(plain_button_text_style(), TextStyle::Button);
    }

    #[test]
    fn progress_rail_uses_shared_low_alpha_border_color() {
        assert_eq!(progress_rail_color(), theme::SEPARATOR);
        assert_eq!(
            progress_rail_color().to_srgba_unmultiplied(),
            [255, 255, 255, 46]
        );
    }

    #[test]
    fn progress_is_clamped_and_remains_monochrome_and_rectangular() {
        assert_eq!(displayed_progress_fraction(-0.5), 0.0);
        assert_eq!(displayed_progress_fraction(f32::NAN), 0.0);
        assert_eq!(displayed_progress_fraction(0.61), 0.61);
        assert_eq!(displayed_progress_fraction(1.4), 1.0);
        assert_eq!(theme::PROGRESS, theme::FOREGROUND);
        assert_eq!(theme::PROGRESS_RADIUS, 0);
        assert_eq!(layout::PROGRESS_TRACK_HEIGHT, 4.0);

        egui::__run_test_ui(|ui| {
            let response = progress_bar(ui, 1.0);
            assert_eq!(response.rect.height(), layout::PROGRESS_TRACK_HEIGHT);
        });
    }

    #[test]
    fn destructive_state_mapping_matches_quiet_focus_exception() {
        let idle = theme::control_colors(
            theme::ControlTone::Destructive,
            theme::InteractionState::Idle,
        );
        let hovered = theme::control_colors(
            theme::ControlTone::Destructive,
            theme::InteractionState::Hovered,
        );
        let pressed = theme::control_colors(
            theme::ControlTone::Destructive,
            theme::InteractionState::Pressed,
        );
        let disabled = theme::control_colors(
            theme::ControlTone::Destructive,
            theme::InteractionState::Disabled,
        );

        assert_eq!(idle.background, theme::BACKGROUND);
        assert_eq!(idle.foreground, theme::FOREGROUND);
        assert_eq!(hovered.background, theme::BACKGROUND);
        assert_eq!(hovered.foreground, theme::DANGER_INTERACTIVE);
        assert_eq!(pressed.foreground, theme::DANGER_INTERACTIVE);
        assert_ne!(disabled.foreground, theme::DANGER_INTERACTIVE);
        assert_ne!(disabled.border, theme::DANGER_INTERACTIVE);
    }

    #[test]
    fn primitives_use_shared_font_and_geometry_owners() {
        assert_eq!(button_text_style(), TextStyle::Button);
        assert_eq!(
            fonts::technical_text_style(),
            TextStyle::Name("apollo-technical".into())
        );
        assert_eq!(theme::PRIMARY_CONTROL_HEIGHT, 31.0);
        assert_eq!(theme::COMPACT_CONTROL_HEIGHT, 30.0);
        assert_eq!(theme::SELECTABLE_ROW_MIN_HEIGHT, 34.0);
        assert_eq!(theme::SELECTION_INDICATOR_SIZE, 12.0);
        assert_eq!(theme::CONTROL_RADIUS, theme::CONTAINER_RADIUS);
        assert!(f32::from(theme::CONTROL_RADIUS) * 2.0 < theme::PRIMARY_CONTROL_HEIGHT);
    }

    #[test]
    fn selectable_rows_expose_layout_friendly_minimum_height() {
        egui::__run_test_ui(|ui| {
            install_test_technical_style(ui);
            let shared_minimum = selectable_row(
                ui,
                "Desktop Audio",
                None,
                true,
                SelectionIndicator::Checkbox,
                0.0,
            );
            let target_minimum = selectable_row(
                ui,
                "4K",
                Some("3840 × 2160"),
                true,
                SelectionIndicator::Radio,
                37.0,
            );

            assert_eq!(
                shared_minimum.rect.height(),
                theme::SELECTABLE_ROW_MIN_HEIGHT
            );
            assert_eq!(target_minimum.rect.height(), 37.0);
        });
    }

    #[test]
    fn inline_error_treatment_stays_monochrome() {
        assert_eq!(
            inline_status_color(InlineStatusKind::Error),
            theme::FOREGROUND
        );
        assert_eq!(
            inline_status_color(InlineStatusKind::Status),
            theme::SECONDARY
        );
        assert_ne!(
            inline_status_color(InlineStatusKind::Error),
            theme::DANGER_INTERACTIVE
        );
    }

    fn install_test_technical_style(ui: &mut egui::Ui) {
        ui.style_mut().text_styles.insert(
            fonts::technical_text_style(),
            FontId::new(fonts::TYPOGRAPHY_SCALE.secondary, FontFamily::Monospace),
        );
    }
}

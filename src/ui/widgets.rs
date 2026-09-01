use egui::{
    Align, Align2, Color32, CornerRadius, Frame, InnerResponse, Label, Layout, Popup, Rect,
    Response, RichText, Sense, Sides, Stroke, StrokeKind, TextEdit, TextStyle, Ui, UiBuilder, Vec2,
    WidgetInfo, WidgetType, pos2, vec2,
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

pub struct OutputFilenameRowResponse {
    pub filename: Response,
    pub browse: Response,
}

pub struct SelectableRowAccessoryResponse<R> {
    pub selection: Response,
    pub accessory: R,
    pub layout: layout::TrackRowLayout,
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

pub fn plain_button_intrinsic_size(ui: &Ui, label: &str) -> Vec2 {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        plain_button_text_style().resolve(ui.style()),
        theme::SECONDARY,
    );
    vec2(
        galley.size().x,
        theme::COMPACT_CONTROL_HEIGHT.max(galley.size().y),
    )
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

/// Stable trigger width for a selector whose options may differ in intrinsic text width.
pub fn select_menu_intrinsic_width(ui: &Ui, option_labels: &[&str]) -> f32 {
    option_labels
        .iter()
        .map(|label| {
            button_intrinsic_size(ui, &format!("{label}  ⌄"), true, BUTTON_HORIZONTAL_PADDING).x
        })
        .fold(0.0, f32::max)
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

fn compact_secondary_button_intrinsic_width(ui: &Ui, label: &str) -> f32 {
    button_intrinsic_size(ui, label, true, BUTTON_HORIZONTAL_PADDING).x
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

/// A full-width label paired with one compact control on the trailing edge.
pub fn labeled_control<R>(ui: &mut Ui, label: &str, add_control: impl FnOnce(&mut Ui) -> R) -> R {
    let (_, control) = Sides::new()
        .height(theme::COMPACT_CONTROL_HEIGHT)
        .spacing(theme::SPACING.related)
        .shrink_left()
        .truncate()
        .show(
            ui,
            |ui| ui.label(label),
            |ui| trailing_control(ui, add_control),
        );
    control
}

/// Apply the shared ready-surface inset to a trailing control or action group.
pub fn trailing_control<R>(ui: &mut Ui, add_control: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(theme::READY_TRAILING_INSET);
    add_control(ui)
}

/// Compact Apollo-styled switch with native checkbox accessibility semantics.
#[must_use]
pub fn switch_control(ui: &mut Ui, label: &str, checked: &mut bool) -> Response {
    let desired_size = vec2(theme::TOGGLE_SWITCH_WIDTH, theme::TOGGLE_SWITCH_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }

    let state = interaction_state(&response);
    let colors = theme::selectable_colors(state, *checked);
    ui.painter().rect(
        rect,
        CornerRadius::same(theme::CONTROL_RADIUS),
        colors.background,
        Stroke::new(theme::SEPARATOR_WIDTH, colors.border),
        StrokeKind::Inside,
    );
    let knob_x = if *checked {
        rect.right() - theme::SPACING.tight - theme::TOGGLE_SWITCH_KNOB_SIZE * 0.5
    } else {
        rect.left() + theme::SPACING.tight + theme::TOGGLE_SWITCH_KNOB_SIZE * 0.5
    };
    let knob = Rect::from_center_size(
        pos2(knob_x, rect.center().y),
        Vec2::splat(theme::TOGGLE_SWITCH_KNOB_SIZE),
    );
    ui.painter().rect_filled(
        knob,
        CornerRadius::same(theme::CONTROL_RADIUS.saturating_sub(1)),
        colors.foreground,
    );
    paint_keyboard_focus(ui, &response, rect);
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, response.enabled(), *checked, label)
    });
    response
}

/// Shared selector popup built from the compact selector trigger and egui's popup owner.
///
/// The popup keeps the trigger as its minimum outer width, while longer option labels may enlarge
/// it. Options should be rendered with [`select_menu_option`] so their hit regions fill the popup's
/// usable width and keep the trigger's centered text alignment and horizontal padding.
pub fn select_menu<R>(
    ui: &mut Ui,
    selected_text: &str,
    option_labels: &[&str],
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let response = select_button(ui, selected_text);
    let inner = show_selector_popup(ui, &response, option_labels, add_contents);

    InnerResponse { inner, response }
}

fn show_selector_popup<R>(
    ui: &mut Ui,
    response: &Response,
    option_labels: &[&str],
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let frame = Frame::popup(ui.style());
    let popup_width = selector_popup_width(ui, response.rect.width(), option_labels, &frame);
    let content_width = (popup_width - frame.total_margin().sum().x).max(0.0);

    Popup::menu(response)
        .width(popup_width)
        .frame(frame)
        .show(|ui| {
            ui.set_min_width(content_width);
            ui.spacing_mut().item_spacing.y = 0.0;
            add_contents(ui)
        })
        .map(|inner_response| inner_response.inner)
}

/// Full-width option row for [`select_menu`].
#[must_use]
pub fn select_menu_option(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let desired_width = ui
        .available_width()
        .max(selector_option_intrinsic_width(ui, label));
    let desired_size = vec2(desired_width, theme::COMPACT_CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    let state = interaction_state(&response);
    let colors = theme::selectable_colors(state, selected);

    ui.painter().rect_filled(rect, 0.0, colors.background);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        button_text_style().resolve(ui.style()),
        colors.foreground,
    );
    paint_keyboard_focus(ui, &response, rect);
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Button, response.enabled(), selected, label)
    });
    response
}

fn selector_popup_width(ui: &Ui, trigger_width: f32, option_labels: &[&str], frame: &Frame) -> f32 {
    let widest_option = option_labels
        .iter()
        .map(|label| selector_option_intrinsic_width(ui, label))
        .fold(0.0_f32, f32::max);
    let option_outer_width = widest_option + frame.total_margin().sum().x;

    trigger_width.max(option_outer_width)
}

fn selector_option_intrinsic_width(ui: &Ui, label: &str) -> f32 {
    button_intrinsic_size(ui, label, true, BUTTON_HORIZONTAL_PADDING).x
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
    paint_selectable_row(
        ui, rect, rect, &response, title, metadata, selected, indicator,
    );
    response
}

/// Selectable row whose trailing control remains independent from the selection hit region.
///
/// At narrow widths the shared layout owner moves the accessory beneath track identity while the
/// selection indicator, title, and metadata retain their normal reading order.
pub fn selectable_row_with_accessory<R>(
    ui: &mut Ui,
    title: &str,
    metadata: Option<&str>,
    selected: bool,
    indicator: SelectionIndicator,
    accessory_width: f32,
    add_accessory: impl FnOnce(&mut Ui) -> R,
) -> SelectableRowAccessoryResponse<R> {
    let accessory_width = finite_nonnegative(accessory_width);
    let plan = layout::track_row_plan(ui.available_width(), accessory_width);
    let desired_size = vec2(finite_nonnegative(ui.available_width()), plan.row_height);
    let (rect, row_response) = ui.allocate_exact_size(desired_size, Sense::hover());
    let text_left = rect.left()
        + theme::SELECTABLE_ROW_HORIZONTAL_PADDING
        + theme::SELECTABLE_ROW_INDICATOR_COLUMN
        + theme::SELECTABLE_ROW_CONTENT_GAP;
    let (selection_rect, accessory_rect) = match plan.layout {
        layout::TrackRowLayout::Inline => {
            let accessory_rect = Rect::from_min_size(
                pos2(
                    rect.right() - theme::SELECTABLE_ROW_HORIZONTAL_PADDING - accessory_width,
                    rect.center().y - theme::COMPACT_CONTROL_HEIGHT * 0.5,
                ),
                vec2(accessory_width, theme::COMPACT_CONTROL_HEIGHT),
            );
            let selection_rect = Rect::from_min_max(
                rect.min,
                pos2(
                    accessory_rect.left() - theme::SPACING.compact,
                    rect.bottom(),
                ),
            );
            (selection_rect, accessory_rect)
        }
        layout::TrackRowLayout::Stacked => {
            let selection_rect = Rect::from_min_max(
                rect.min,
                pos2(rect.right(), rect.top() + theme::SELECTABLE_ROW_MIN_HEIGHT),
            );
            let accessory_rect = Rect::from_min_size(
                pos2(text_left, selection_rect.bottom() + theme::SPACING.compact),
                vec2(accessory_width, theme::COMPACT_CONTROL_HEIGHT),
            );
            (selection_rect, accessory_rect)
        }
    };
    let selection = ui.interact(
        selection_rect,
        row_response.id.with("selection"),
        Sense::click(),
    );

    paint_selectable_row(
        ui,
        rect,
        selection_rect,
        &selection,
        title,
        metadata,
        selected,
        indicator,
    );

    let accessory = ui
        .scope_builder(
            UiBuilder::new()
                .id_salt(row_response.id.with("accessory"))
                .max_rect(accessory_rect)
                .layout(Layout::left_to_right(Align::Center)),
            add_accessory,
        )
        .inner;

    SelectableRowAccessoryResponse {
        selection,
        accessory,
        layout: plan.layout,
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_selectable_row(
    ui: &mut Ui,
    row_rect: Rect,
    selection_rect: Rect,
    response: &Response,
    title: &str,
    metadata: Option<&str>,
    selected: bool,
    indicator: SelectionIndicator,
) {
    let state = interaction_state(response);
    let visuals = selectable_row_visuals(state, selected);

    ui.painter().rect_filled(row_rect, 0.0, visuals.background);
    ui.painter().line_segment(
        [row_rect.left_bottom(), row_rect.right_bottom()],
        Stroke::new(theme::SEPARATOR_WIDTH, theme::SEPARATOR),
    );

    let indicator_rect = Rect::from_center_size(
        pos2(
            selection_rect.left()
                + theme::SELECTABLE_ROW_HORIZONTAL_PADDING
                + theme::SELECTABLE_ROW_INDICATOR_COLUMN * 0.5,
            selection_rect.center().y,
        ),
        Vec2::splat(theme::SELECTION_INDICATOR_SIZE),
    );
    paint_selection_indicator(ui, indicator_rect, indicator, selected, visuals);

    let text_left = selection_rect.left()
        + theme::SELECTABLE_ROW_HORIZONTAL_PADDING
        + theme::SELECTABLE_ROW_INDICATOR_COLUMN
        + theme::SELECTABLE_ROW_CONTENT_GAP;
    let text_right = selection_rect.right() - theme::SELECTABLE_ROW_HORIZONTAL_PADDING;
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
        pos2(text_left, selection_rect.top()),
        pos2(title_right, selection_rect.bottom()),
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
            pos2(text_right, selection_rect.center().y),
            Align2::RIGHT_CENTER,
            metadata,
            metadata_font,
            theme::SECONDARY_FAINT,
        );
    }

    paint_keyboard_focus(ui, response, selection_rect);
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Button, response.enabled(), selected, title)
    });
}

/// Editable output filename stem with an authoritative, non-editable extension suffix.
///
/// The caller owns path state. This widget edits only the filename stem and keeps long text clipped
/// to the available single-line field, allowing egui to scroll the edit cursor horizontally.
#[must_use]
pub fn filename_field(ui: &mut Ui, stem: &mut String, extension: &str, width: f32) -> Response {
    let suffix = format!(".{extension}");
    ui.add_sized(
        [finite_nonnegative(width), theme::COMPACT_CONTROL_HEIGHT],
        TextEdit::singleline(stem)
            .suffix(suffix)
            .font(fonts::technical_text_style())
            .vertical_align(Align::Center)
            .text_color(theme::FOREGROUND)
            .clip_text(true),
    )
}

/// Shared responsive operation output row: selector, editable filename, and Browse.
#[must_use]
pub fn operation_output_row(
    ui: &mut Ui,
    stem: &mut String,
    extension: &str,
    browse_label: &str,
    selector_width: f32,
    add_selector: impl FnMut(&mut Ui),
) -> OutputFilenameRowResponse {
    operation_output_row_impl(
        ui,
        stem,
        extension,
        browse_label,
        selector_width,
        add_selector,
    )
}

/// Shared layout plan used by every operation output-row renderer.
pub fn operation_output_row_plan(
    ui: &Ui,
    selector_width: f32,
    browse_label: &str,
) -> layout::OutputRowPlan {
    layout::output_row_plan(
        ui.available_width(),
        Some(selector_width),
        compact_secondary_button_intrinsic_width(ui, browse_label),
    )
}

fn operation_output_row_impl(
    ui: &mut Ui,
    stem: &mut String,
    extension: &str,
    browse_label: &str,
    selector_width: f32,
    mut add_selector: impl FnMut(&mut Ui),
) -> OutputFilenameRowResponse {
    let plan = operation_output_row_plan(ui, selector_width, browse_label);

    match plan.layout {
        layout::OutputRowLayout::Inline => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACING.compact;
                output_selector_slot(ui, selector_width, &mut add_selector);
                let filename = filename_field(ui, stem, extension, plan.filename_width);
                let browse = compact_secondary_button(ui, browse_label);
                OutputFilenameRowResponse { filename, browse }
            })
            .inner
        }
        layout::OutputRowLayout::Stacked => {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = theme::SPACING.compact;
                output_selector_slot(ui, selector_width, &mut add_selector);
                let filename = filename_field(ui, stem, extension, plan.filename_width);
                let browse = ui
                    .allocate_ui_with_layout(
                        vec2(plan.filename_width, theme::COMPACT_CONTROL_HEIGHT),
                        Layout::right_to_left(Align::Center),
                        |ui| compact_secondary_button(ui, browse_label),
                    )
                    .inner;
                OutputFilenameRowResponse { filename, browse }
            })
            .inner
        }
    }
}

fn output_selector_slot(ui: &mut Ui, selector_width: f32, add_selector: &mut impl FnMut(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(
        vec2(selector_width, theme::COMPACT_CONTROL_HEIGHT),
        Sense::hover(),
    );
    let mut selector_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    add_selector(&mut selector_ui);
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

pub fn inline_status_height(ui: &Ui, text: &str, wrap_width: f32) -> f32 {
    ui.painter()
        .layout(
            text.to_owned(),
            TextStyle::Body.resolve(ui.style()),
            theme::FOREGROUND,
            wrap_width.max(0.0),
        )
        .size()
        .y
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
            theme::operation_mode_colors(state, true)
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
        BUTTON_HORIZONTAL_PADDING, InlineStatusKind, SelectionIndicator, button_intrinsic_size,
        button_text_style, displayed_progress_fraction, filename_field, inline_status,
        inline_status_color, inline_status_height, labeled_control, mode_button,
        mode_button_intrinsic_size, operation_output_row, plain_button_text_style,
        primary_button_enabled, progress_bar, progress_rail_color, select_button,
        select_menu_intrinsic_width, select_menu_option, selectable_row, selectable_row_visuals,
        selectable_row_with_accessory, selector_popup_width, show_selector_popup, switch_control,
    };
    use crate::ui::{fonts, layout, theme};

    fn shape_rect_fill(shape: &egui::Shape, rect: egui::Rect) -> Option<egui::Color32> {
        match shape {
            egui::Shape::Rect(shape)
                if shape.rect == rect && shape.fill != egui::Color32::TRANSPARENT =>
            {
                Some(shape.fill)
            }
            egui::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| shape_rect_fill(shape, rect))
            }
            _ => None,
        }
    }

    fn shape_rect_stroke(shape: &egui::Shape, rect: egui::Rect) -> Option<egui::Color32> {
        match shape {
            egui::Shape::Rect(shape)
                if shape.rect == rect && shape.stroke != egui::Stroke::NONE =>
            {
                Some(shape.stroke.color)
            }
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| shape_rect_stroke(shape, rect)),
            _ => None,
        }
    }

    fn shape_text_color(shape: &egui::Shape, text: &str) -> Option<egui::Color32> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.job.text == text => shape
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.color),
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| shape_text_color(shape, text)),
            _ => None,
        }
    }

    #[test]
    fn editable_filename_field_stays_bounded_with_long_text() {
        let context = egui::Context::default();
        super::fonts::install(&context);
        let original = "very long output filename ".repeat(12);
        let mut stem = original.clone();

        let output = context.run_ui(Default::default(), |ui| {
            let response = filename_field(ui, &mut stem, "mp4", 120.0);

            assert_eq!(response.rect.width(), 120.0);
            assert_eq!(response.rect.height(), theme::COMPACT_CONTROL_HEIGHT);
        });
        output.drop_without_applying_deltas();

        assert_eq!(stem, original);
    }

    #[test]
    fn labeled_controls_keep_the_control_on_the_trailing_edge() {
        egui::__run_test_ui(|ui| {
            ui.set_width(320.0);
            let control_rect = labeled_control(ui, "Integrated loudness", |ui| {
                ui.allocate_exact_size(egui::vec2(86.0, 30.0), Sense::click())
                    .0
            });

            assert_eq!(control_rect.width(), 86.0);
            assert_eq!(control_rect.right(), 320.0 - theme::READY_TRAILING_INSET);
        });
    }

    #[test]
    fn switch_control_uses_shared_geometry_and_checkbox_semantics() {
        egui::__run_test_ui(|ui| {
            ui.set_width(320.0);
            let mut checked = false;
            let response = switch_control(ui, "Normalize output loudness", &mut checked);

            assert!(response.sense.senses_click());
            assert!(response.sense.is_focusable());
            assert_eq!(response.rect.width(), theme::TOGGLE_SWITCH_WIDTH);
            assert_eq!(response.rect.height(), theme::TOGGLE_SWITCH_HEIGHT);
            assert!(!checked);
        });
    }

    #[test]
    fn editable_filename_text_is_vertically_centered() {
        let context = egui::Context::default();
        super::fonts::install(&context);
        let mut stem = String::from("base");
        let mut field_rect = egui::Rect::NOTHING;

        let output = context.run_ui(Default::default(), |ui| {
            field_rect = filename_field(ui, &mut stem, "mp4", 160.0).rect;
        });

        let text_center_y = output
            .shapes
            .iter()
            .find_map(|clipped| text_shape_center_y(&clipped.shape, "base"))
            .expect("filename text should be painted");
        assert!((text_center_y - field_rect.center().y).abs() <= 0.5);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn output_row_anchors_browse_and_gives_long_filename_only_flexible_width() {
        let context = egui::Context::default();
        super::fonts::install(&context);
        let mut stem = "very long output filename ".repeat(12);
        let mut row_rect = egui::Rect::NOTHING;
        let mut selector_rect = egui::Rect::NOTHING;

        let output = context.run_ui(Default::default(), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(564.0, 100.0), egui::Sense::hover());
            row_rect = rect;
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            let selector_width = select_menu_intrinsic_width(&child, &["MP4", "MKV"]);
            let row = operation_output_row(
                &mut child,
                &mut stem,
                "mp4",
                "Browse",
                selector_width,
                |ui| selector_rect = select_button(ui, "MP4").rect,
            );

            assert!(row.filename.rect.width() >= layout::OUTPUT_FILENAME_MIN_INLINE_WIDTH);
            assert!(selector_rect.right() <= row.filename.rect.left());
            assert!(row.filename.rect.right() <= row.browse.rect.left());
            assert!((row.browse.rect.right() - rect.right()).abs() <= 0.5);
        });
        output.drop_without_applying_deltas();

        assert_eq!(stem, "very long output filename ".repeat(12));
        assert!(selector_rect.left() >= row_rect.left());
    }

    #[test]
    fn minimum_content_width_keeps_every_operation_selector_usable() {
        let content_width =
            layout::MINIMUM_CONTENT_SIZE.x - f32::from(layout::CONTENT_PADDING) * 2.0;
        let mut saw_inline = false;
        let mut saw_stacked = false;
        for options in [
            &["MP4", "MKV"][..],
            &["H.264", "H.265"][..],
            &["Original (copy)", "AAC", "FLAC"][..],
            &["PNG", "JPEG"][..],
        ] {
            let context = egui::Context::default();
            super::fonts::install(&context);
            let mut stem = String::from("recording-output");
            let output = context.run_ui(Default::default(), |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(content_width, 140.0), egui::Sense::hover());
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                );
                let selector_width = select_menu_intrinsic_width(&child, options);
                let mut selector_rect = egui::Rect::NOTHING;
                let row = operation_output_row(
                    &mut child,
                    &mut stem,
                    "mp4",
                    "Browse",
                    selector_width,
                    |ui| selector_rect = select_button(ui, options[0]).rect,
                );

                assert!(selector_rect.width() <= selector_width + 0.5);
                assert!(selector_rect.width() > 0.0);
                let inline = selector_rect.right() <= row.filename.rect.left()
                    && row.filename.rect.right() <= row.browse.rect.left();
                let stacked = selector_rect.bottom() <= row.filename.rect.top()
                    && row.filename.rect.bottom() <= row.browse.rect.top();
                assert!(inline || stacked);
                saw_inline |= inline;
                saw_stacked |= stacked;
                assert!(
                    (row.browse.rect.right() - rect.right()).abs() <= 0.5,
                    "options={options:?} browse={:?} row={rect:?}",
                    row.browse.rect
                );
            });
            output.drop_without_applying_deltas();
        }
        assert!(saw_inline);
        assert!(saw_stacked);
    }

    #[test]
    fn narrow_output_row_stacks_selector_filename_and_browse_without_overlap() {
        let context = egui::Context::default();
        super::fonts::install(&context);
        let mut stem = String::from("recording-mix");
        let mut selector_rect = egui::Rect::NOTHING;

        let output = context.run_ui(Default::default(), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(300.0, 140.0), egui::Sense::hover());
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            let selector_width = select_menu_intrinsic_width(&child, &["MP4", "MKV"]);
            let row = operation_output_row(
                &mut child,
                &mut stem,
                "mp4",
                "Browse",
                selector_width,
                |ui| selector_rect = select_button(ui, "MP4").rect,
            );

            assert!((row.filename.rect.width() - rect.width()).abs() <= 0.5);
            assert!(selector_rect.bottom() <= row.filename.rect.top());
            assert!(row.filename.rect.bottom() <= row.browse.rect.top());
            assert!((row.browse.rect.right() - rect.right()).abs() <= 0.5);
        });
        output.drop_without_applying_deltas();
    }

    fn text_shape_center_y(shape: &egui::Shape, needle: &str) -> Option<f32> {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                Some(text.pos.y + text.galley.size().y * 0.5)
            }
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| text_shape_center_y(shape, needle)),
            _ => None,
        }
    }

    #[test]
    fn selector_popups_match_triggers_and_long_labels_can_expand_them() {
        let context = egui::Context::default();
        let output = context.run_ui(Default::default(), |ui| {
            let frame = egui::Frame::popup(ui.style());
            let mp4_trigger =
                button_intrinsic_size(ui, "MP4  ⌄", true, BUTTON_HORIZONTAL_PADDING).x;
            let codec_trigger =
                button_intrinsic_size(ui, "H.264  ⌄", true, BUTTON_HORIZONTAL_PADDING).x;

            let mp4_popup = selector_popup_width(ui, mp4_trigger, &["MP4", "MKV"], &frame);
            let codec_popup = selector_popup_width(ui, codec_trigger, &["H.264", "H.265"], &frame);
            let compact = selector_popup_width(ui, 0.0, &["MP4", "MKV"], &frame);
            let long = selector_popup_width(
                ui,
                0.0,
                &["MP4", "A deliberately longer container label"],
                &frame,
            );

            assert!(mp4_popup >= mp4_trigger);
            assert!(codec_popup >= codec_trigger);
            assert!(long > compact);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn selector_options_fill_the_popup_usable_width() {
        egui::__run_test_ui(|ui| {
            ui.set_width(180.0);
            let option = select_menu_option(ui, "H.264", false);

            assert_eq!(option.rect.width(), 180.0);
            assert_eq!(option.rect.height(), theme::COMPACT_CONTROL_HEIGHT);
            assert_eq!(option.sense, Sense::click());
        });
    }

    #[test]
    fn opening_selector_popup_does_not_reflow_adjacent_layout() {
        let context = egui::Context::default();
        let output = context.run_ui(Default::default(), |ui| {
            ui.horizontal(|ui| {
                let spacing = ui.spacing().item_spacing.x;
                let trigger = select_button(ui, "MP4");
                let expected_adjacent_left = trigger.rect.right() + spacing;

                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&trigger));
                let popup = show_selector_popup(
                    ui,
                    &trigger,
                    &["MP4", "A deliberately longer container label"],
                    |ui| select_menu_option(ui, "MP4", true),
                );
                assert!(popup.is_some());

                let adjacent = ui.label("Output");
                assert!((adjacent.rect.left() - expected_adjacent_left).abs() < 0.01);
            });
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn selector_popup_frame_uses_shared_theme_roles() {
        let context = egui::Context::default();
        theme::install(&context);
        let output = context.run_ui(Default::default(), |ui| {
            let frame = egui::Frame::popup(ui.style());

            assert_eq!(frame.fill, theme::BACKGROUND);
            assert_eq!(frame.stroke.color, theme::SEPARATOR);
            assert_eq!(frame.stroke.width, theme::SEPARATOR_WIDTH);
            assert_eq!(
                frame.corner_radius,
                egui::CornerRadius::same(theme::CONTAINER_RADIUS),
            );
            assert_eq!(frame.shadow, egui::Shadow::NONE);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn selector_option_states_stay_on_shared_monochrome_roles() {
        let idle = theme::selectable_colors(theme::InteractionState::Idle, false);
        let hovered = theme::selectable_colors(theme::InteractionState::Hovered, false);
        let pressed = theme::selectable_colors(theme::InteractionState::Pressed, false);
        let selected = theme::selectable_colors(theme::InteractionState::Idle, true);

        assert_eq!(idle.background, theme::BACKGROUND);
        assert_eq!(hovered.background, theme::HOVER);
        assert_eq!(pressed.background, theme::PRESSED);
        assert_eq!(selected.background, theme::SELECTION);
        assert_eq!(selected.foreground, theme::BACKGROUND);
    }

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
    fn selected_operation_keeps_unselected_control_geometry() {
        egui::__run_test_ui(|ui| {
            let unselected = mode_button(ui, "Audio Mix", false);
            let selected = mode_button(ui, "Audio Mix", true);

            assert_eq!(unselected.rect.size(), selected.rect.size());
        });
    }

    #[test]
    fn disabled_selected_operation_keeps_geometry_and_rejects_interaction() {
        egui::__run_test_ui(|ui| {
            ui.disable();
            let unselected = mode_button(ui, "Audio Mix", false);
            let selected = mode_button(ui, "Audio Mix", true);

            assert_eq!(unselected.rect.size(), selected.rect.size());
            assert!(!selected.enabled());
            assert!(!selected.clicked());
        });
    }

    #[test]
    fn disabled_selected_operation_paints_the_selected_palette() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut unselected_rect = egui::Rect::NOTHING;
        let mut selected_rect = egui::Rect::NOTHING;
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.disable();
            unselected_rect = mode_button(ui, "Upscale", false).rect;
            selected_rect = mode_button(ui, "Audio Mix", true).rect;
        });
        output.textures_delta.clear();

        let find_fill = |rect| {
            output
                .shapes
                .iter()
                .find_map(|shape| shape_rect_fill(&shape.shape, rect))
        };
        let find_stroke = |rect| {
            output
                .shapes
                .iter()
                .find_map(|shape| shape_rect_stroke(&shape.shape, rect))
        };
        let find_text = |text| {
            output
                .shapes
                .iter()
                .find_map(|shape| shape_text_color(&shape.shape, text))
        };

        assert_ne!(find_fill(selected_rect), find_fill(unselected_rect));
        assert_ne!(find_stroke(selected_rect), find_stroke(unselected_rect));
        assert_ne!(find_text("Audio Mix"), find_text("Upscale"));
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
    fn track_accessory_is_independent_and_moves_below_identity_at_narrow_width() {
        let context = egui::Context::default();
        fonts::install(&context);
        let output = context.run_ui(Default::default(), |ui| {
            for (width, expected_layout) in [
                (564.0, layout::TrackRowLayout::Inline),
                (344.0, layout::TrackRowLayout::Stacked),
            ] {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 100.0), Sense::hover());
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                );
                let row = selectable_row_with_accessory(
                    &mut child,
                    "Desktop Audio",
                    Some("Stereo"),
                    true,
                    SelectionIndicator::Checkbox,
                    theme::TRACK_GAIN_CONTROL_WIDTH,
                    |ui| {
                        ui.allocate_exact_size(
                            egui::vec2(
                                theme::TRACK_GAIN_CONTROL_WIDTH,
                                theme::COMPACT_CONTROL_HEIGHT,
                            ),
                            Sense::click(),
                        )
                        .0
                    },
                );

                assert_eq!(row.layout, expected_layout);
                assert_eq!(row.selection.sense, Sense::click());
                assert!(row.selection.rect.intersect(row.accessory).is_negative());
                match expected_layout {
                    layout::TrackRowLayout::Inline => {
                        assert!(row.selection.rect.right() < row.accessory.left());
                        assert_eq!(
                            row.accessory.right(),
                            rect.right() - theme::READY_TRAILING_INSET
                        );
                    }
                    layout::TrackRowLayout::Stacked => {
                        assert!(row.selection.rect.bottom() < row.accessory.top());
                    }
                }
            }
        });
        output.drop_without_applying_deltas();
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

    #[test]
    fn inline_status_height_matches_wrapped_label_at_current_width() {
        let context = egui::Context::default();
        super::fonts::install(&context);
        theme::install(&context);
        let text = "A source stream cannot be copied into MP4. Use MKV instead.";

        let output = context.run_ui(Default::default(), |ui| {
            for width in [344.0, 800.0] {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 200.0), egui::Sense::hover());
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                );
                let measured = inline_status_height(&child, text, child.available_width());
                let rendered = inline_status(&mut child, text, InlineStatusKind::Error)
                    .rect
                    .height();
                assert!(
                    (measured - rendered).abs() <= 0.5,
                    "width={width}, measured={measured}, rendered={rendered}"
                );
            }
        });
        output.drop_without_applying_deltas();
    }

    fn install_test_technical_style(ui: &mut egui::Ui) {
        ui.style_mut().text_styles.insert(
            fonts::technical_text_style(),
            FontId::new(fonts::TYPOGRAPHY_SCALE.secondary, FontFamily::Monospace),
        );
    }
}

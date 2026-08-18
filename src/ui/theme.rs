use egui::style::{HandleShape, ImeComposition, TextCursorStyle, WidgetVisuals, Widgets};
use egui::{Color32, Context, CornerRadius, Shadow, Stroke, Style, Vec2, Visuals};

// Color values are the implementation of `.project/design/COLOR.md`.
// White-alpha roles deliberately avoid a separate neutral-gray palette.
pub const BACKGROUND: Color32 = Color32::BLACK;
pub const FOREGROUND: Color32 = Color32::WHITE;
pub const SECONDARY: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 168);
pub const SEPARATOR: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 46);
pub const HOVER: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 14);
pub const PRESSED: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 28);
pub const DISABLED: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 87);
pub const SELECTION: Color32 = FOREGROUND;
pub const FOCUS: Color32 = FOREGROUND;
pub const PROGRESS: Color32 = FOREGROUND;
pub const DANGER_INTERACTIVE: Color32 = Color32::from_rgb(0x7E, 0x30, 0x30);

pub const CONTROL_RADIUS: u8 = 2;
pub const CONTAINER_RADIUS: u8 = 2;
pub const PROGRESS_RADIUS: u8 = 0;
pub const SEPARATOR_WIDTH: f32 = 1.0;
pub const PRIMARY_CONTROL_HEIGHT: f32 = 31.0;
pub const COMPACT_CONTROL_HEIGHT: f32 = 30.0;

const _: () = assert!(CONTROL_RADIUS <= 2);
const _: () = assert!(CONTAINER_RADIUS <= 2);
const _: () = assert!(PROGRESS_RADIUS == 0);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacingScale {
    pub tight: f32,
    pub compact: f32,
    pub related: f32,
    pub section: f32,
    pub spacious: f32,
}

pub const SPACING: SpacingScale = SpacingScale {
    tight: 4.0,
    compact: 7.0,
    related: 11.0,
    section: 24.0,
    spacious: 40.0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeRole {
    Background,
    Foreground,
    Secondary,
    Separator,
    Hover,
    Pressed,
    Disabled,
    Selection,
    Focus,
    Progress,
    DangerInteractive,
}

impl ThemeRole {
    pub const ALL: [Self; 11] = [
        Self::Background,
        Self::Foreground,
        Self::Secondary,
        Self::Separator,
        Self::Hover,
        Self::Pressed,
        Self::Disabled,
        Self::Selection,
        Self::Focus,
        Self::Progress,
        Self::DangerInteractive,
    ];

    pub const fn color(self) -> Color32 {
        match self {
            Self::Background => BACKGROUND,
            Self::Foreground => FOREGROUND,
            Self::Secondary => SECONDARY,
            Self::Separator => SEPARATOR,
            Self::Hover => HOVER,
            Self::Pressed => PRESSED,
            Self::Disabled => DISABLED,
            Self::Selection => SELECTION,
            Self::Focus => FOCUS,
            Self::Progress => PROGRESS,
            Self::DangerInteractive => DANGER_INTERACTIVE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlTone {
    Standard,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionState {
    Idle,
    Hovered,
    Pressed,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlColors {
    pub background: Color32,
    pub foreground: Color32,
    pub border: Color32,
}

pub const fn control_colors(tone: ControlTone, state: InteractionState) -> ControlColors {
    match (tone, state) {
        (_, InteractionState::Disabled) => ControlColors {
            background: BACKGROUND,
            foreground: DISABLED,
            border: DISABLED,
        },
        (ControlTone::Destructive, InteractionState::Hovered | InteractionState::Pressed) => {
            ControlColors {
                background: DANGER_INTERACTIVE,
                foreground: FOREGROUND,
                border: DANGER_INTERACTIVE,
            }
        }
        (_, InteractionState::Idle) => ControlColors {
            background: BACKGROUND,
            foreground: FOREGROUND,
            border: SEPARATOR,
        },
        (_, InteractionState::Hovered) => ControlColors {
            background: HOVER,
            foreground: FOREGROUND,
            border: FOCUS,
        },
        (_, InteractionState::Pressed) => ControlColors {
            background: PRESSED,
            foreground: FOREGROUND,
            border: FOCUS,
        },
    }
}

pub fn install(context: &Context) {
    context.all_styles_mut(apply_to_style);
}

fn apply_to_style(style: &mut Style) {
    style.visuals = monochrome_visuals();
    style.spacing.item_spacing = Vec2::splat(SPACING.related);
    style.spacing.button_padding = Vec2::new(SPACING.related, SPACING.compact);
    style.spacing.interact_size = Vec2::splat(PRIMARY_CONTROL_HEIGHT);
}

fn monochrome_visuals() -> Visuals {
    let mut visuals = Visuals::dark();

    visuals.override_text_color = None;
    visuals.weak_text_alpha = 0.66;
    visuals.weak_text_color = Some(SECONDARY);
    visuals.widgets = monochrome_widgets();
    visuals.selection.bg_fill = SELECTION;
    visuals.selection.stroke = Stroke::new(SEPARATOR_WIDTH, BACKGROUND);
    visuals.ime_composition = ImeComposition {
        active_underline_stroke: Stroke::new(2.0, FOCUS),
        inactive_underline_stroke: Stroke::new(SEPARATOR_WIDTH, SEPARATOR),
        ..visuals.ime_composition
    };
    visuals.hyperlink_color = FOREGROUND;
    visuals.faint_bg_color = HOVER;
    visuals.extreme_bg_color = BACKGROUND;
    visuals.text_edit_bg_color = Some(BACKGROUND);
    visuals.code_bg_color = BACKGROUND;
    visuals.warn_fg_color = FOREGROUND;
    visuals.error_fg_color = FOREGROUND;
    visuals.window_corner_radius = CornerRadius::same(CONTAINER_RADIUS);
    visuals.window_shadow = Shadow::NONE;
    visuals.window_fill = BACKGROUND;
    visuals.window_stroke = Stroke::new(SEPARATOR_WIDTH, SEPARATOR);
    visuals.window_highlight_topmost = false;
    visuals.menu_corner_radius = CornerRadius::same(CONTAINER_RADIUS);
    visuals.panel_fill = BACKGROUND;
    visuals.popup_shadow = Shadow::NONE;
    visuals.text_cursor = TextCursorStyle {
        stroke: Stroke::new(2.0, FOCUS),
        ..visuals.text_cursor
    };
    visuals.button_frame = true;
    visuals.collapsing_header_frame = false;
    visuals.indent_has_left_vline = true;
    visuals.striped = false;
    visuals.slider_trailing_fill = false;
    visuals.handle_shape = HandleShape::Rect { aspect_ratio: 0.75 };
    visuals.disabled_alpha = 0.34;

    visuals
}

fn monochrome_widgets() -> Widgets {
    let radius = CornerRadius::same(CONTROL_RADIUS);
    let border = Stroke::new(SEPARATOR_WIDTH, SEPARATOR);
    let focus_border = Stroke::new(SEPARATOR_WIDTH, FOCUS);
    let foreground = Stroke::new(SEPARATOR_WIDTH, FOREGROUND);

    Widgets {
        noninteractive: WidgetVisuals {
            bg_fill: BACKGROUND,
            weak_bg_fill: BACKGROUND,
            bg_stroke: border,
            corner_radius: CornerRadius::same(CONTAINER_RADIUS),
            fg_stroke: foreground,
            expansion: 0.0,
        },
        inactive: WidgetVisuals {
            bg_fill: BACKGROUND,
            weak_bg_fill: BACKGROUND,
            bg_stroke: border,
            corner_radius: radius,
            fg_stroke: foreground,
            expansion: 0.0,
        },
        hovered: WidgetVisuals {
            bg_fill: HOVER,
            weak_bg_fill: HOVER,
            bg_stroke: focus_border,
            corner_radius: radius,
            fg_stroke: foreground,
            expansion: 0.0,
        },
        active: WidgetVisuals {
            bg_fill: PRESSED,
            weak_bg_fill: PRESSED,
            bg_stroke: focus_border,
            corner_radius: radius,
            fg_stroke: foreground,
            expansion: 0.0,
        },
        open: WidgetVisuals {
            bg_fill: PRESSED,
            weak_bg_fill: PRESSED,
            bg_stroke: focus_border,
            corner_radius: radius,
            fg_stroke: foreground,
            expansion: 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use egui::{Color32, Style};

    use super::{
        BACKGROUND, COMPACT_CONTROL_HEIGHT, ControlTone, DANGER_INTERACTIVE, DISABLED, FOCUS,
        FOREGROUND, HOVER, InteractionState, PRESSED, PRIMARY_CONTROL_HEIGHT, PROGRESS,
        PROGRESS_RADIUS, SECONDARY, SELECTION, SEPARATOR, SEPARATOR_WIDTH, SPACING, ThemeRole,
        apply_to_style, control_colors, monochrome_visuals,
    };

    #[test]
    fn project_color_spec_is_encoded_exactly() {
        assert_eq!(BACKGROUND, Color32::from_rgb(0x00, 0x00, 0x00));
        assert_eq!(FOREGROUND, Color32::from_rgb(0xFF, 0xFF, 0xFF));
        assert_eq!(SECONDARY.to_srgba_unmultiplied(), [255, 255, 255, 168]);
        assert_eq!(SEPARATOR.to_srgba_unmultiplied(), [255, 255, 255, 46]);
        assert_eq!(HOVER.to_srgba_unmultiplied(), [255, 255, 255, 14]);
        assert_eq!(PRESSED.to_srgba_unmultiplied(), [255, 255, 255, 28]);
        assert_eq!(DISABLED.to_srgba_unmultiplied(), [255, 255, 255, 87]);
        assert_eq!(DANGER_INTERACTIVE, Color32::from_rgb(0x7E, 0x30, 0x30));
    }

    #[test]
    fn theme_roles_are_limited_to_the_project_spec() {
        assert_eq!(
            ThemeRole::ALL,
            [
                ThemeRole::Background,
                ThemeRole::Foreground,
                ThemeRole::Secondary,
                ThemeRole::Separator,
                ThemeRole::Hover,
                ThemeRole::Pressed,
                ThemeRole::Disabled,
                ThemeRole::Selection,
                ThemeRole::Focus,
                ThemeRole::Progress,
                ThemeRole::DangerInteractive,
            ]
        );
    }

    #[test]
    fn progress_selection_and_focus_are_monochrome() {
        assert_eq!(ThemeRole::Progress.color(), FOREGROUND);
        assert_eq!(ThemeRole::Selection.color(), FOREGROUND);
        assert_eq!(ThemeRole::Focus.color(), FOREGROUND);
        assert_eq!(PROGRESS, FOREGROUND);
        assert_eq!(SELECTION, FOREGROUND);
        assert_eq!(FOCUS, FOREGROUND);
    }

    #[test]
    fn destructive_idle_is_monochrome() {
        let standard = control_colors(ControlTone::Standard, InteractionState::Idle);
        let destructive = control_colors(ControlTone::Destructive, InteractionState::Idle);

        assert_eq!(destructive, standard);
        assert_eq!(destructive.background, BACKGROUND);
        assert_eq!(destructive.foreground, FOREGROUND);
    }

    #[test]
    fn destructive_hover_and_press_use_only_interactive_danger() {
        for state in [InteractionState::Hovered, InteractionState::Pressed] {
            let colors = control_colors(ControlTone::Destructive, state);

            assert_eq!(colors.background, DANGER_INTERACTIVE);
            assert_eq!(colors.foreground, FOREGROUND);
            assert_eq!(colors.border, DANGER_INTERACTIVE);
        }
    }

    #[test]
    fn destructive_disabled_returns_to_monochrome_disabled_state() {
        let standard = control_colors(ControlTone::Standard, InteractionState::Disabled);
        let destructive = control_colors(ControlTone::Destructive, InteractionState::Disabled);

        assert_eq!(destructive, standard);
        assert_eq!(destructive.foreground, DISABLED);
        assert_ne!(destructive.background, DANGER_INTERACTIVE);
        assert_ne!(destructive.border, DANGER_INTERACTIVE);
    }

    #[test]
    fn geometry_remains_minimal_and_progress_is_square() {
        let visuals = monochrome_visuals();

        for widget in [
            visuals.widgets.noninteractive,
            visuals.widgets.inactive,
            visuals.widgets.hovered,
            visuals.widgets.active,
            visuals.widgets.open,
        ] {
            assert!(widget.corner_radius.average() <= 2.0);
        }
        assert!(visuals.window_corner_radius.average() <= 2.0);
        assert!(visuals.menu_corner_radius.average() <= 2.0);
        assert_eq!(std::hint::black_box(PROGRESS_RADIUS), 0);
        assert_eq!(std::hint::black_box(SEPARATOR_WIDTH), 1.0);
        assert_eq!(std::hint::black_box(PRIMARY_CONTROL_HEIGHT), 31.0);
        assert_eq!(std::hint::black_box(COMPACT_CONTROL_HEIGHT), 30.0);
    }

    #[test]
    fn installed_visuals_remove_egui_accent_colors_and_shadows() {
        let visuals = monochrome_visuals();
        let mut colors = vec![
            visuals.hyperlink_color,
            visuals.faint_bg_color,
            visuals.extreme_bg_color,
            visuals
                .text_edit_bg_color
                .expect("text edit fill is explicit"),
            visuals.code_bg_color,
            visuals.warn_fg_color,
            visuals.error_fg_color,
            visuals.window_fill,
            visuals.window_stroke.color,
            visuals.panel_fill,
            visuals.selection.bg_fill,
            visuals.selection.stroke.color,
            visuals.ime_composition.active_underline_stroke.color,
            visuals.ime_composition.inactive_underline_stroke.color,
            visuals.text_cursor.stroke.color,
        ];

        for widget in [
            visuals.widgets.noninteractive,
            visuals.widgets.inactive,
            visuals.widgets.hovered,
            visuals.widgets.active,
            visuals.widgets.open,
        ] {
            colors.extend([
                widget.bg_fill,
                widget.weak_bg_fill,
                widget.bg_stroke.color,
                widget.fg_stroke.color,
            ]);
        }

        assert!(colors.into_iter().all(is_black_or_white_alpha));
        assert_eq!(visuals.window_shadow, egui::Shadow::NONE);
        assert_eq!(visuals.popup_shadow, egui::Shadow::NONE);
    }

    #[test]
    fn style_uses_central_spacing_and_control_metrics() {
        let mut style = Style::default();
        apply_to_style(&mut style);

        assert_eq!(
            style.spacing.item_spacing,
            egui::Vec2::splat(SPACING.related)
        );
        assert_eq!(style.spacing.button_padding.x, SPACING.related);
        assert_eq!(style.spacing.button_padding.y, SPACING.compact);
        assert_eq!(style.spacing.interact_size.y, PRIMARY_CONTROL_HEIGHT);
    }

    fn is_black_or_white_alpha(color: Color32) -> bool {
        let [r, g, b, _] = color.to_srgba_unmultiplied();
        (r == 0 && g == 0 && b == 0) || (r == 255 && g == 255 && b == 255)
    }
}

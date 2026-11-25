//! Shared stateless GPUI controls. Callbacks belong to the consuming view.
use std::{borrow::Cow, time::Duration};

use gpui::{
    Animation, AnimationExt, App, AssetSource, ClickEvent, Div, ElementId, Pixels, Role,
    SharedString, Stateful, TextRun, Transformation, Window, WindowTextSystem, div, percentage,
    prelude::*, px, rgba, svg,
};

use super::design::{DESTRUCTIVE_BORDER, METRICS as M, Palette, SPINNER_CYCLE_MS, TextRole};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Standard,
    Quiet,
    Destructive,
}

/// Bundled glyphs for button icons.
#[derive(Clone, Copy)]
pub enum ActionIcon {
    Back,
    Settings,
    ChevronDown,
}

impl ActionIcon {
    fn path(self) -> &'static str {
        match self {
            Self::Back => "icons/arrow-left.svg",
            Self::Settings => "icons/settings.svg",
            Self::ChevronDown => "icons/chevron-down.svg",
        }
    }
}

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Disabled and busy controls cannot carry an activation callback.
pub enum ActionState {
    Enabled(Click),
    Disabled,
    Busy,
    Selected(Click),
}

/// Busy and alternate labels use the same measured action slot.
pub struct Button<'a> {
    pub id: ElementId,
    pub label: &'a str,
    pub tone: Tone,
    pub state: ActionState,
    pub alternate_labels: &'a [&'a str],
}

impl<'a> Button<'a> {
    pub fn new(id: impl Into<ElementId>, label: &'a str, tone: Tone) -> Self {
        Self {
            id: id.into(),
            label,
            tone,
            state: ActionState::Disabled,
            alternate_labels: &[],
        }
    }

    pub fn measured_width(&self, window: &Window) -> Pixels {
        self.measured_width_with(window.text_system(), None)
    }

    fn horizontal_padding(&self, trailing_icon: bool) -> f32 {
        if self.tone == Tone::Quiet {
            M.tight
        } else if trailing_icon {
            M.compact
        } else {
            M.related
        }
    }

    fn minimum_height(&self) -> Pixels {
        let minimum = if self.tone == Tone::Primary {
            M.primary_height
        } else {
            M.compact_height
        };
        px(minimum.max(TextRole::Control.resolve().line_height + 2. * (M.tight + M.rule)))
    }

    fn measured_width_with(&self, text: &WindowTextSystem, icon: Option<ActionIcon>) -> Pixels {
        let icon_width = if icon.is_some() {
            M.icon_size + M.tight
        } else {
            0.
        };
        (label_width(
            text,
            self.label,
            self.alternate_labels,
            self.horizontal_padding(icon.is_some()),
        ) + px(icon_width))
        .max(px(
            M.spinner_size + 2. * (self.horizontal_padding(icon.is_some()) + M.rule)
        ))
    }

    pub fn render(self, palette: Palette, window: &Window) -> Stateful<Div> {
        self.render_with_icon(palette, window, None, None)
    }

    /// Icon-only actions retain the same semantics and focus treatment as text buttons.
    pub fn render_icon(self, icon: ActionIcon, palette: Palette, window: &Window) -> Stateful<Div> {
        let label = self.label.to_owned();
        self.render_with_icon(palette, window, None, Some(icon))
            .tooltip(tooltip(label, palette))
    }

    fn render_with_icon(
        self,
        palette: Palette,
        window: &Window,
        icon: Option<ActionIcon>,
        icon_only: Option<ActionIcon>,
    ) -> Stateful<Div> {
        let busy = matches!(&self.state, ActionState::Busy);
        let width = (busy || !self.alternate_labels.is_empty())
            .then(|| self.measured_width_with(window.text_system(), icon));
        let height = self.minimum_height();
        let width = if icon_only.is_some() {
            Some(height)
        } else {
            width
        };
        let padding = if icon_only.is_some() {
            M.tight
        } else {
            self.horizontal_padding(icon.is_some())
        };
        let selected = matches!(&self.state, ActionState::Selected(_));
        let click = match self.state {
            ActionState::Enabled(click) | ActionState::Selected(click) => Some(click),
            ActionState::Disabled | ActionState::Busy => None,
        };
        let enabled = click.is_some();
        let primary = self.tone == Tone::Primary && !selected;
        let quiet = self.tone == Tone::Quiet;
        let foreground = if !enabled && !busy {
            palette.disabled()
        } else if primary {
            palette.background
        } else {
            palette.foreground
        };
        let background = if selected {
            palette.pressed()
        } else if primary && (enabled || busy) {
            palette.foreground
        } else {
            palette.background
        };
        let border = if !enabled && !busy {
            palette.disabled()
        } else if selected || primary {
            palette.foreground
        } else {
            palette.border()
        };
        let mut element = TextRole::Control.apply(
            div()
                .id(self.id)
                .role(Role::Button)
                .aria_label(self.label.to_owned())
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .min_h(height)
                .px(px(padding))
                .py(px(M.tight))
                .rounded(px(M.radius))
                // The focused border replaces the idle border; compensate padding to preserve size.
                .border(px(M.rule))
                .border_color(rgba(if quiet { 0 } else { border }))
                .bg(rgba(background))
                .text_color(rgba(foreground))
                .whitespace_nowrap(),
        );
        element = element.min_w(width.unwrap_or(px(M.spinner_size + 2. * (padding + M.rule))));
        if busy {
            return element.aria_description("In progress").child(spinner_icon(
                "busy-icon",
                foreground,
                true,
            ));
        }
        let element = if let Some(icon) = icon_only {
            element.w(height).child(
                svg()
                    .path(icon.path())
                    .size(px(16.))
                    .flex_shrink_0()
                    .text_color(rgba(foreground)),
            )
        } else {
            element
                .child(self.label.to_owned())
                .children(icon.map(|icon| {
                    svg()
                        .path(icon.path())
                        .ml(px(M.tight))
                        .size(px(M.icon_size))
                        .flex_shrink_0()
                        .text_color(rgba(foreground))
                }))
        };
        let Some(click) = click else {
            return element;
        };
        let hover = if primary {
            palette.primary_hover()
        } else if selected {
            palette.pressed()
        } else {
            palette.hover()
        };
        let pressed = if primary {
            palette.primary_pressed()
        } else {
            palette.pressed()
        };
        let destructive = self.tone == Tone::Destructive;
        let element = element
            .tab_index(0)
            .cursor_pointer()
            .hover(move |style| {
                let style = style.bg(rgba(hover));
                let style = if destructive {
                    style.border_color(rgba(DESTRUCTIVE_BORDER))
                } else {
                    style
                };
                if quiet && icon_only.is_none() {
                    style.underline()
                } else {
                    style
                }
            })
            .active(move |style| {
                let style = style.bg(rgba(pressed));
                if destructive {
                    style.border_color(rgba(DESTRUCTIVE_BORDER))
                } else {
                    style
                }
            });
        let focus_style = move |style: gpui::StyleRefinement| {
            // The underline remains visible even if a destructive hover overrides the border color.
            let style = if destructive {
                style.underline()
            } else {
                style
            };
            style
                .border(px(M.focus_width))
                .px(px(padding - (M.focus_width - M.rule)))
                .py(px(M.tight - (M.focus_width - M.rule)))
                .border_color(rgba(if primary {
                    palette.background
                } else {
                    palette.foreground
                }))
        };
        let element = if icon.is_some() {
            element.focus_visible(focus_style)
        } else {
            element.focus(focus_style)
        };
        element.on_click(click)
    }
}

fn label_width(
    text: &WindowTextSystem,
    label: &str,
    alternatives: &[&str],
    padding: f32,
) -> Pixels {
    reserve_label_width(label, alternatives, padding, |label| {
        text_width(text, TextRole::Control, label)
    })
}

pub(super) fn text_width(text: &WindowTextSystem, role: TextRole, label: &str) -> Pixels {
    let typography = role.resolve();
    let run = TextRun {
        len: label.len(),
        font: typography.font(),
        color: rgba(Palette::DARK.foreground).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    text.layout_line(label, px(typography.size), &[run], None)
        .width
        .ceil()
}

fn reserve_label_width(
    label: &str,
    alternatives: &[&str],
    padding: f32,
    mut measure: impl FnMut(&str) -> Pixels,
) -> Pixels {
    std::iter::once(label)
        .chain(alternatives.iter().copied())
        // GPUI rounds intrinsic text width up to logical pixels.
        .map(|label| measure(label).ceil())
        .fold(px(0.), |a, b| a.max(b))
        + px(2. * (padding + M.rule))
}

/// Icon and visible label share the regular action contract. Keep the label for keyboard and assistive access.
pub fn icon_action(
    button: Button<'_>,
    icon: ActionIcon,
    palette: Palette,
    window: &Window,
) -> Stateful<Div> {
    button.render_with_icon(palette, window, Some(icon), None)
}

pub fn selector(items: impl IntoIterator<Item = Stateful<Div>>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(M.compact))
        .children(items)
}

pub fn selectable_row(button: Button<'_>, palette: Palette, window: &Window) -> Stateful<Div> {
    button
        .render(palette, window)
        .w_full()
        .min_h(px(M.row_height))
        .justify_start()
}

pub fn heading(label: impl Into<gpui::SharedString>) -> Div {
    TextRole::SectionHeading.apply(div()).child(label.into())
}

pub fn page_title(label: impl Into<SharedString>) -> Div {
    TextRole::PageTitle.apply(div()).child(label.into())
}

pub fn separator(palette: Palette) -> Div {
    div()
        .w_full()
        .h(px(M.rule))
        .flex_shrink_0()
        .bg(rgba(palette.separator()))
}

pub fn status(
    label: impl Into<gpui::SharedString>,
    palette: Palette,
    details: Option<Stateful<Div>>,
) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(M.compact))
        .whitespace_nowrap()
        .text_color(rgba(palette.foreground))
        .child(TextRole::Status.apply(div()).child(label.into()))
        .children(details)
}

pub fn inline_message(message: impl Into<gpui::SharedString>, palette: Palette) -> Div {
    TextRole::Body
        .apply(div())
        .text_color(rgba(palette.secondary()))
        .w_full()
        .min_w_0()
        .whitespace_normal()
        .child(message.into())
}

pub fn empty_state(title: &str, message: &str, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .gap(px(M.related))
        .child(heading(title.to_owned()).text_color(rgba(palette.foreground)))
        .child(inline_message(message.to_owned(), palette))
}

const SPINNER_PATH: &str = "icons/spinner.svg";

/// Keep bundled glyphs in the executable, independent of the working directory.
pub struct Assets;
const ICON_ASSETS: &[(&str, &[u8])] = &[
    (
        SPINNER_PATH,
        include_bytes!("../../assets/icons/spinner.svg"),
    ),
    (
        "icons/arrow-left.svg",
        include_bytes!("../../assets/icons/arrow-left.svg"),
    ),
    (
        "icons/settings.svg",
        include_bytes!("../../assets/icons/settings.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../../assets/icons/chevron-down.svg"),
    ),
];
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICON_ASSETS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICON_ASSETS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| (*name).into())
            .collect())
    }
}

/// The shared icon has a stable animation ID scoped to its owning element.
/// GPUI also stops animation when the application enables reduced motion.
fn spinner_icon(id: impl Into<ElementId>, color: u32, animate: bool) -> gpui::AnyElement {
    let icon = svg()
        .path(SPINNER_PATH)
        .size(px(M.spinner_size))
        .flex_shrink_0()
        .text_color(rgba(color));
    if animate {
        icon.with_animation(
            id,
            Animation::new(Duration::from_millis(SPINNER_CYCLE_MS)).repeat(),
            |icon, delta| icon.with_transformation(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
    } else {
        icon.into_any_element()
    }
}

/// Render only for outstanding work. The name remains available to assistive technology.
pub fn spinner(
    id: impl Into<ElementId>,
    label: &str,
    palette: Palette,
    animate: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::ProgressIndicator)
        .aria_label(label.to_owned())
        .aria_description("In progress")
        .flex()
        .items_center()
        .justify_center()
        .gap(px(M.compact))
        .min_h(px(M.compact_height))
        .child(spinner_icon("spin", palette.foreground, animate))
        .child(TextRole::Status.apply(div()).child(label.to_owned()))
}

/// Center first-content loading within the same flex region used by its ready view.
pub fn centered_spinner(
    id: impl Into<ElementId>,
    label: &str,
    palette: Palette,
    animate: bool,
) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .min_w_0()
        .min_h(px(M.compact_height))
        .items_center()
        .justify_center()
        .child(spinner(id, label, palette, animate))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProgressState<'a> {
    Idle,
    Waiting(&'a str),
    Active {
        label: &'a str,
        fraction: Option<f32>,
    },
}

fn determinate_fraction(fraction: Option<f32>) -> Option<f32> {
    fraction
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0., 1.))
}

/// Allocate the same footprint for idle, waiting, indeterminate and determinate progress.
pub fn progress(
    id: impl Into<ElementId>,
    state: ProgressState<'_>,
    palette: Palette,
    animate: bool,
) -> Div {
    let region = div()
        .flex()
        .flex_col()
        .w_full()
        .flex_shrink_0()
        .h(px(M.progress_height))
        .justify_center();
    match state {
        ProgressState::Idle => region,
        ProgressState::Waiting(label) => region.child(
            TextRole::Status
                .apply(div())
                .text_center()
                .child(label.to_owned()),
        ),
        ProgressState::Active { label, fraction } => match determinate_fraction(fraction) {
            None => region.child(spinner(id, label, palette, animate)),
            Some(fraction) => region.child(
                div()
                    .id(id)
                    .role(Role::ProgressIndicator)
                    .aria_label(label.to_owned())
                    .aria_numeric_value(f64::from(fraction) * 100.)
                    .aria_min_numeric_value(0.)
                    .aria_max_numeric_value(100.)
                    .flex()
                    .flex_col()
                    .gap(px(M.tight))
                    .w_full()
                    .child(
                        TextRole::Status
                            .apply(div())
                            .text_center()
                            .child(format!("{label} · {}%", (fraction * 100.).floor() as u32)),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(M.tight))
                            .bg(rgba(palette.separator()))
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui::relative(fraction))
                                    .bg(rgba(palette.foreground)),
                            ),
                    ),
            ),
        },
    }
}

pub fn action_bar(palette: Palette, actions: impl IntoIterator<Item = Stateful<Div>>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(M.action_inset))
        .child(separator(palette))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_end()
                .gap(px(M.action_gap))
                .children(actions),
        )
}

/// Shared GPUI tooltip builder for concise action labels and truncated content.
pub(super) fn tooltip(
    text: impl Into<String>,
    palette: Palette,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tooltip(text.clone(), palette)).into()
}

struct Tooltip(String, Palette);

impl gpui::Render for Tooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(340.))
            .p(px(M.compact))
            .rounded(px(M.radius))
            .bg(rgba(self.1.background))
            .border(px(M.rule))
            .border_color(rgba(self.1.border()))
            .child(
                TextRole::Secondary
                    .apply(div())
                    .text_color(rgba(self.1.foreground))
                    .whitespace_normal()
                    .child(self.0.clone()),
            )
    }
}

/// Shared confirmation surface; the caller owns dismissal, focus and action validity.
pub fn confirmation(
    title: &str,
    message: &str,
    actions: impl IntoIterator<Item = Stateful<Div>>,
    palette: Palette,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(M.related))
        .w_full()
        .child(empty_state(title, message, palette))
        .child(action_bar(palette, actions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_font_requests_medium_and_tabular_figures() {
        let font = TextRole::Control.resolve().font();
        assert_eq!(font.family.as_str(), "Geist");
        assert_eq!(font.weight, gpui::FontWeight::MEDIUM);
        assert!(
            font.features
                .tag_value_list()
                .iter()
                .any(|(tag, value)| tag == "tnum" && *value == 1)
        );
        let brand = TextRole::Wordmark.resolve().font();
        assert_eq!(brand.family.as_str(), "Fira Code");
        assert_eq!(brand.weight, gpui::FontWeight::NORMAL);
        assert!(brand.features.tag_value_list().is_empty());
    }

    #[test]
    fn reserved_width_uses_widest_complete_label_and_rounds_up() {
        let measure = |label: &str| match label {
            "Open files" => px(61.2),
            "Change file" => px(89.5),
            "変更ファイル 🗂" => px(116.1),
            _ => panic!("unexpected label"),
        };
        let alternatives = ["Open files", "Change file", "変更ファイル 🗂"];
        for label in alternatives {
            let width = reserve_label_width(label, &alternatives, M.related, measure);
            assert_eq!(width, px(117. + 2. * (M.related + M.rule)));
        }
        assert_eq!(
            reserve_label_width("Open files", &[], M.related, measure),
            px(62. + 2. * (M.related + M.rule))
        );
    }

    #[test]
    fn queue_label_reservation_is_stable_until_the_count_needs_more_space() {
        // Distinct sample widths exercise the reservation policy, not a particular font's metrics.
        let measure = |label: &str| match label {
            "Queue" => px(40.3),
            "Queue (1)" => px(60.8),
            "Queue (99)" => px(69.4),
            "Queue (1000)" => px(84.1),
            _ => panic!("unexpected label"),
        };
        let alternatives = ["Queue", "Queue (99)"];
        let reserved = reserve_label_width("Queue", &alternatives, M.related, measure);
        for count in [0, 1, 99] {
            let label = crate::ui::queue_count_label(count);
            assert_eq!(
                reserve_label_width(&label, &alternatives, M.related, measure),
                reserved
            );
        }
        let label = crate::ui::queue_count_label(1000);
        assert!(reserve_label_width(&label, &alternatives, M.related, measure) > reserved);
    }

    #[test]
    fn progress_uses_only_finite_measured_fractions() {
        for fraction in [
            None,
            Some(f32::NAN),
            Some(f32::INFINITY),
            Some(f32::NEG_INFINITY),
        ] {
            assert_eq!(determinate_fraction(fraction), None);
        }
        for (input, expected) in [(-0.1, 0.), (0., 0.), (0.42, 0.42), (1.1, 1.)] {
            assert_eq!(determinate_fraction(Some(input)), Some(expected));
        }
    }

    #[test]
    fn button_height_fits_line_and_padding_in_every_state() {
        for tone in [
            Tone::Primary,
            Tone::Standard,
            Tone::Quiet,
            Tone::Destructive,
        ] {
            let mut button = Button::new("test", "Label", tone);
            let height = button.minimum_height();
            assert!(
                height >= px(TextRole::Control.resolve().line_height + 2. * (M.tight + M.rule))
            );
            for state in [
                ActionState::Busy,
                ActionState::Enabled(Box::new(|_, _, _| {})),
                ActionState::Selected(Box::new(|_, _, _| {})),
            ] {
                button.state = state;
                assert_eq!(button.minimum_height(), height);
            }
        }
    }
}

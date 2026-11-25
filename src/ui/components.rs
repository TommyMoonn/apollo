//! Shared stateless GPUI controls. Callbacks belong to the consuming view.
use std::{borrow::Cow, time::Duration};

use gpui::{
    Animation, AnimationExt, App, AssetSource, ClickEvent, Div, ElementId, Font, FontFallbacks,
    FontWeight, Pixels, SharedString, Stateful, TextRun, Transformation, Window, div, font,
    percentage, prelude::*, px, rgba, svg,
};

use super::design::{DESTRUCTIVE_BORDER, FONT_FAMILY, METRICS as M, Palette, SPINNER_CYCLE_MS};

pub fn ui_font() -> Font {
    let mut font = font(FONT_FAMILY);
    font.fallbacks = Some(FontFallbacks::from_fonts(vec![
        "Segoe UI".into(),
        "Segoe UI Symbol".into(),
    ]));
    font
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Standard,
    Quiet,
    Destructive,
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
        let padding = if self.tone == Tone::Quiet {
            M.tight
        } else {
            M.related
        };
        label_width(window, self.label, self.alternate_labels, padding)
    }

    pub fn render(self, palette: Palette, window: &Window) -> Stateful<Div> {
        let width = (!self.alternate_labels.is_empty()).then(|| self.measured_width(window));
        let selected = matches!(&self.state, ActionState::Selected(_));
        let busy = matches!(&self.state, ActionState::Busy);
        let click = match self.state {
            ActionState::Enabled(click) | ActionState::Selected(click) => Some(click),
            ActionState::Disabled | ActionState::Busy => None,
        };
        let enabled = click.is_some();
        let primary = self.tone == Tone::Primary && !selected;
        let quiet = self.tone == Tone::Quiet;
        let padding = if quiet { M.tight } else { M.related };
        let foreground = if busy {
            palette.secondary()
        } else if !enabled {
            palette.disabled()
        } else if primary {
            palette.background
        } else {
            palette.foreground
        };
        let background = if selected {
            palette.pressed()
        } else if primary && enabled {
            palette.foreground
        } else {
            palette.background
        };
        let border = if !enabled {
            palette.disabled()
        } else if selected || primary {
            palette.foreground
        } else {
            palette.border()
        };
        let mut element = div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .min_h(px(if primary {
                M.primary_height
            } else {
                M.compact_height
            }))
            .px(px(padding))
            .py(px(M.tight))
            .rounded(px(M.radius))
            // The focused border replaces the idle border; compensate padding to preserve size.
            .border(px(M.rule))
            .border_color(rgba(if quiet { 0 } else { border }))
            .bg(rgba(background))
            .text_color(rgba(foreground))
            .font(ui_font())
            .text_size(px(M.body))
            .whitespace_nowrap();
        if let Some(width) = width {
            element = element.min_w(width);
        }
        let element = element.child(self.label.to_owned());
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
                if quiet { style.underline() } else { style }
            })
            .active(move |style| {
                let style = style.bg(rgba(pressed));
                if destructive {
                    style.border_color(rgba(DESTRUCTIVE_BORDER))
                } else {
                    style
                }
            })
            .focus(move |style| {
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
            });
        element.on_click(click)
    }
}

fn label_width(window: &Window, label: &str, alternatives: &[&str], padding: f32) -> Pixels {
    std::iter::once(label)
        .chain(alternatives.iter().copied())
        .map(|label| {
            let run = TextRun {
                len: label.len(),
                font: ui_font(),
                color: rgba(Palette::DARK.foreground).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .layout_line(label, px(M.body), &[run], None)
                .width
        })
        .fold(px(0.), |a, b| a.max(b))
        + px(2. * (padding + M.rule))
}

/// Icon and visible label share the regular action contract. Keep the label for keyboard and assistive access.
pub fn icon_action(
    button: Button<'_>,
    icon: &str,
    palette: Palette,
    window: &Window,
) -> Stateful<Div> {
    // The icon is supplemental; the action never relies on an unlabeled glyph.
    button.render(palette, window).child(
        div()
            .ml(px(M.tight))
            .w(px(M.body))
            .flex_shrink_0()
            .text_center()
            .child(icon.to_owned()),
    )
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
    div()
        .text_size(px(M.body))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label.into())
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
        .text_size(px(M.body))
        .text_color(rgba(palette.foreground))
        .child(label.into())
        .children(details)
}

pub fn inline_message(message: impl Into<gpui::SharedString>, palette: Palette) -> Div {
    div()
        .text_color(rgba(palette.secondary()))
        .text_size(px(M.body))
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
        .child(status(title.to_owned(), palette, None))
        .child(inline_message(message.to_owned(), palette))
}

/// Pending text does not schedule idle frames. An indeterminate operation never shows a percentage.
pub fn pending(label: &str, palette: Palette) -> Div {
    status(format!("{label}…"), palette, None).min_h(px(M.compact_height))
}

const SPINNER_PATH: &str = "icons/spinner.svg";

/// Keep the one bundled icon in the executable, rather than relying on the current directory.
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok((path == SPINNER_PATH)
            .then(|| Cow::Borrowed(include_bytes!("../../assets/icons/spinner.svg").as_slice())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(if SPINNER_PATH.starts_with(path) {
            vec![SPINNER_PATH.into()]
        } else {
            vec![]
        })
    }
}

/// Only callers with actual pending work render this indicator. The static variant respects reduced motion.
pub fn spinner(id: impl Into<ElementId>, label: &str, palette: Palette, animate: bool) -> Div {
    let icon = svg()
        .path(SPINNER_PATH)
        .size(px(M.body))
        .flex_shrink_0()
        .text_color(rgba(palette.foreground));
    let row = div()
        .flex()
        .items_center()
        .gap(px(M.compact))
        .min_h(px(M.compact_height));
    let row = if animate {
        row.child(icon.with_animation(
            id,
            Animation::new(Duration::from_millis(SPINNER_CYCLE_MS)).repeat(),
            |icon, delta| icon.with_transformation(Transformation::rotate(percentage(delta))),
        ))
    } else {
        row.child(icon)
    };
    row.child(label.to_owned())
}

pub fn progress(fraction: f32, palette: Palette) -> Div {
    if !fraction.is_finite() {
        return pending("Processing", palette);
    }
    let fraction = fraction.clamp(0., 1.);
    div()
        .flex()
        .flex_col()
        .gap(px(M.tight))
        .child(format!("{:.0}%", fraction * 100.))
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
        )
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

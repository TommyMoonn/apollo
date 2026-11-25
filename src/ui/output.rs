//! Shared container, filename and destination controls for GPUI operation forms.
use super::{
    components::{self, Button},
    design::{METRICS as M, Palette, TextRole},
    text_input::TextInput,
};
use gpui::{Div, Entity, Stateful, Window, div, prelude::*, px, rgba};
use std::path::Path;

pub(super) fn output_row(
    choices: [Button<'_>; 2],
    filename: Entity<TextInput>,
    extension: &str,
    directory: &Path,
    browse: Stateful<Div>,
    palette: Palette,
    window: &Window,
) -> Div {
    let directory = if directory.as_os_str().is_empty() {
        Path::new(".")
    } else {
        directory
    };
    let directory = directory.to_string_lossy().into_owned();
    let description = format!("Destination: {directory}");
    let narrow = window.viewport_size().width < px(560.);
    let filename = div()
        .flex()
        .items_center()
        .gap(px(M.tight))
        .min_w_0()
        .flex_1()
        .when(narrow, |row| row.w_full())
        .child(div().flex_1().min_w_0().child(filename))
        .child(
            TextRole::Control
                .apply(div())
                .flex_shrink_0()
                .child(format!(".{extension}")),
        );
    let choices = components::selector(choices.map(|choice| choice.render(palette, window)));
    let controls = div()
        .flex()
        .gap(px(M.compact))
        .w_full()
        .min_w_0()
        .when(narrow, |row| row.flex_col().items_start())
        .when(!narrow, |row| row.items_center())
        .child(choices)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(M.compact))
                .min_w_0()
                .flex_1()
                .when(narrow, |row| row.flex_col().items_start().w_full())
                .child(filename)
                .child(browse),
        );
    div()
        .flex()
        .flex_col()
        .gap(px(M.compact))
        .w_full()
        .min_w_0()
        .child(components::heading("Output"))
        .child(controls)
        .child(
            TextRole::Secondary
                .apply(div())
                .text_color(rgba(palette.secondary()))
                .id("output-directory")
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .aria_label(format!("Destination: {directory}"))
                .tooltip(move |_, cx| {
                    cx.new(|_| components::PathTooltip(directory.clone(), palette))
                        .into()
                })
                .child(description),
        )
}

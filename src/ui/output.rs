//! Shared filename and destination controls for GPUI operation forms.
use super::{
    components,
    design::{METRICS as M, Palette, TextRole},
    text_input::TextInput,
};
use gpui::{Div, Entity, Stateful, Window, div, prelude::*, px};
use std::path::Path;

pub(super) struct OutputRow<'a> {
    pub format: Div,
    pub filename: Entity<TextInput>,
    pub extension: &'a str,
    pub extensions: &'a [&'a str],
    pub directory: &'a Path,
    pub browse: Stateful<Div>,
}

impl OutputRow<'_> {
    pub fn render(self, palette: Palette, window: &Window) -> Div {
        let directory = if self.directory.as_os_str().is_empty() {
            Path::new(".")
        } else {
            self.directory
        };
        let destination = format!("Destination: {}", directory.to_string_lossy());
        let browse = components::with_tooltip(
            self.browse.aria_description(destination.clone()),
            destination,
            palette,
        );
        let extension_width = self
            .extensions
            .iter()
            .copied()
            .chain(std::iter::once(self.extension))
            .map(|extension| {
                components::text_width(
                    window.text_system(),
                    TextRole::Control,
                    &format!(".{extension}"),
                )
            })
            .fold(px(0.), |a, b| a.max(b));
        let narrow = window.viewport_size().width < px(560.);
        let filename = div()
            .flex()
            .items_center()
            .gap(px(M.tight))
            .min_w_0()
            .flex_1()
            .when(narrow, |row| row.w_full())
            .child(div().flex_1().min_w_0().child(self.filename))
            .child(
                TextRole::Control
                    .apply(div())
                    .w(extension_width)
                    .flex_shrink_0()
                    .child(format!(".{}", self.extension)),
            );
        let controls = div()
            .flex()
            .gap(px(M.compact))
            .w_full()
            .min_w_0()
            .when(narrow, |row| row.flex_col().items_start())
            .when(!narrow, |row| row.items_center())
            .child(self.format)
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
    }
}

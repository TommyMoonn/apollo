//! Source-first GPUI presentation. Actions are supplied by the root.
use gpui::{Div, Stateful, div, prelude::*, px, rgba};

use super::{
    design::{METRICS as M, Palette, TextRole},
    workspace::WorkspacePresentation,
};

pub fn wordmark() -> Div {
    TextRole::Wordmark.apply(div()).child("apollo")
}

pub fn landing(open: Stateful<Div>, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(M.compact))
        .w_full()
        .max_w(px(super::REFERENCE_CONTENT_DIMENSIONS.0 - 2. * M.inset))
        .child(open)
        .child(
            div()
                .flex()
                .items_center()
                .min_h(px(TextRole::Status.resolve().line_height))
                .text_color(rgba(palette.secondary()))
                .child(
                    TextRole::Secondary
                        .apply(div())
                        .child("or drop videos anywhere"),
                ),
        )
}

/// Reserve the cancellation row above and below the center so it does not pull the indicator off center.
pub fn inspecting(cancel: Stateful<Div>, palette: Palette, animate: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .min_h(px(3. * M.compact_height + 2. * M.compact))
        .items_center()
        .justify_center()
        .gap(px(M.compact))
        .child(div().h(px(M.compact_height)).flex_shrink_0())
        .child(super::components::spinner(
            "source-inspection",
            "Checking file",
            palette,
            animate,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(M.compact_height))
                .flex_shrink_0()
                .child(cancel),
        )
}

pub fn summary(presentation: WorkspacePresentation, path: String, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .w_full()
        .gap(px(M.tight))
        .child(
            TextRole::SectionHeading
                .apply(div())
                .id("source-name")
                .w_full()
                .truncate()
                .child(presentation.file_name.clone())
                .tooltip(move |_, cx| {
                    cx.new(|_| SourcePath(presentation.file_name.clone(), palette))
                        .into()
                }),
        )
        .children(presentation.metadata.map(|metadata| {
            TextRole::Secondary
                .apply(div())
                .text_color(rgba(palette.secondary()))
                .truncate()
                .child(metadata)
        }))
        .child(
            TextRole::Secondary
                .apply(div())
                .id("source-path")
                .w_full()
                .text_ellipsis_middle()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(rgba(palette.secondary()))
                .child(path.clone())
                .tooltip(move |_, cx| cx.new(|_| SourcePath(path.clone(), palette)).into()),
        )
}

struct SourcePath(String, Palette);

impl gpui::Render for SourcePath {
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
                    .text_color(rgba(self.1.secondary()))
                    .whitespace_normal()
                    .child(self.0.clone()),
            )
    }
}

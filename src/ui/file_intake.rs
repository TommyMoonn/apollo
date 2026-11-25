//! Source-first GPUI presentation. Actions are supplied by the root.
use gpui::{Div, FontWeight, Stateful, div, prelude::*, px, rgba};

use super::{
    components,
    design::{METRICS as M, Palette},
    workspace::WorkspacePresentation,
};

pub const WORDMARK_FAMILY: &str = "Fira Code";

pub fn wordmark() -> Div {
    div()
        .font_family(WORDMARK_FAMILY)
        .text_size(px(M.title))
        .child("apollo")
}

pub fn landing(
    open: Stateful<Div>,
    cancel: Option<Stateful<Div>>,
    inspecting: bool,
    palette: Palette,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(M.compact))
        .w_full()
        .min_h(px(M.primary_height
            + M.compact_height
            + M.secondary
            + M.compact * 2.))
        .child(open)
        .child(
            div()
                .text_size(px(M.secondary))
                .text_color(rgba(palette.secondary()))
                .child(if inspecting {
                    "Checking file…"
                } else {
                    "or drop media files anywhere"
                }),
        )
        .child(
            div()
                .flex()
                .justify_center()
                .min_h(px(M.compact_height))
                .children(cancel),
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
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .w_full()
                .truncate()
                .child(presentation.file_name),
        )
        .children(presentation.metadata.map(|metadata| {
            div()
                .text_size(px(M.secondary))
                .text_color(rgba(palette.secondary()))
                .child(metadata)
        }))
        .child(
            div()
                .id("source-path")
                .w_full()
                .text_ellipsis_middle()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(px(M.secondary))
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
            .child(components::inline_message(self.0.clone(), self.1))
    }
}

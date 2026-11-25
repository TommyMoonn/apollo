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

pub fn summary(
    presentation: WorkspacePresentation,
    path: String,
    actions: Div,
    palette: Palette,
) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .w_full()
        .gap(px(M.tight))
        .child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap(px(M.related))
                .child(
                    TextRole::SectionHeading
                        .apply(div())
                        .id("source-name")
                        .flex_1()
                        .min_w(px(120.))
                        .truncate()
                        .child(presentation.file_name.clone())
                        .tooltip(move |_, cx| {
                            cx.new(|_| {
                                super::components::PathTooltip(
                                    presentation.file_name.clone(),
                                    palette,
                                )
                            })
                            .into()
                        }),
                )
                .child(actions.flex_shrink_0().max_w_full().ml_auto()),
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
                .tooltip(move |_, cx| {
                    cx.new(|_| super::components::PathTooltip(path.clone(), palette))
                        .into()
                }),
        )
}

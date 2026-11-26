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
    back: Option<Stateful<Div>>,
    actions: Div,
    palette: Palette,
) -> Div {
    let name = presentation.file_name;
    let identity = div()
        .flex()
        .items_center()
        .gap(px(M.compact))
        .flex_1()
        .min_w(px(160.))
        .children(back)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(M.tight))
                .child(super::components::with_tooltip(
                    TextRole::SectionHeading
                        .apply(div())
                        .id("source-name")
                        .truncate()
                        .child(name.clone()),
                    name,
                    palette,
                ))
                .children(presentation.metadata.map(|metadata| {
                    TextRole::Secondary
                        .apply(div())
                        .text_color(rgba(palette.secondary()))
                        .truncate()
                        .child(metadata)
                })),
        );
    div()
        .flex()
        .items_center()
        .flex_wrap()
        .min_w_0()
        .w_full()
        .gap(px(M.related))
        .child(identity)
        .child(actions.flex_shrink_0().max_w_full().ml_auto())
}

//! Common GPUI page layout. Header and footer stay fixed when there is usable
//! body space; otherwise the page itself is the only scroll owner.
use std::{cell::Cell, rc::Rc};

use gpui::{Div, Stateful, Window, div, prelude::*, px};

use super::{
    components::{ActionIcon, Button},
    design::{METRICS as M, Palette},
};

pub fn navigation_actions(
    queue: Button<'_>,
    settings: Button<'_>,
    palette: Palette,
    window: &Window,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .justify_center()
        .items_center()
        .gap(px(M.action_gap))
        .child(queue.render(palette, window))
        .child(settings.render_icon(ActionIcon::Settings, palette, window))
}

/// Keep the actual measured chrome, not estimates based on filename lengths or
/// selector breakpoints. A resize may require one extra frame to switch modes.
/// Both modes have identical chrome width and only one scroll container.
pub fn page(
    header: Div,
    body: Div,
    footer: Div,
    whole_page_scroll: Rc<Cell<bool>>,
) -> Stateful<Div> {
    let scroll_page = whole_page_scroll.get();
    let body = div()
        .id("shell-body")
        .flex_1()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .child(body.flex_auto().flex_shrink_0());
    let body = if scroll_page {
        body.flex_auto().flex_shrink_0()
    } else {
        body.min_h_0().overflow_y_scroll()
    };
    let content = div()
        .flex()
        .flex_col()
        .w_full()
        .p(px(M.inset))
        .gap(px(M.related))
        .child(header.flex_shrink_0().w_full())
        .child(body)
        .child(footer.flex_shrink_0().w_full())
        .on_children_prepainted(move |bounds, window, cx| {
            let [header, _, footer] = bounds.as_slice() else {
                return;
            };
            let chrome =
                header.size.height + footer.size.height + px(2. * (M.inset + M.related + M.rule));
            // Reserve enough space for two control rows and their spacing.
            let fallback =
                window.viewport_size().height < chrome + px(2. * M.primary_height + M.related);
            if whole_page_scroll.replace(fallback) != fallback {
                window.defer(cx, |window, _| window.refresh());
            }
        });
    let page = div().id("shell-page").size_full();
    if scroll_page {
        page.overflow_y_scroll().child(content.min_h_full())
    } else {
        page.child(content.h_full())
    }
}

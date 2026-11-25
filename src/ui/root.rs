//! GPUI application entry and root view. Work remains unavailable until its owners are wired.

use std::{borrow::Cow, cell::Cell, process::ExitCode, rc::Rc};

use gpui::{
    App, Application as GpuiApplication, Bounds, Context, FocusHandle, FontWeight, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, rgba, size,
};

use crate::app::{Application, NavigationState};
use crate::native;

use super::{
    MINIMUM_CONTENT_DIMENSIONS, REFERENCE_CONTENT_DIMENSIONS,
    components::{self, Button, Tone},
    design::{FONT_FAMILY, METRICS as M, Palette},
};

struct ApolloRoot {
    core: Application,
    focus: FocusHandle,
}

impl Render for ApolloRoot {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let message = match self.core.navigation() {
            NavigationState::NoSource => "Opening files is unavailable during the UI migration.",
            _ => "This view is unavailable during the UI migration.",
        };

        let palette = Palette::DARK;
        let open = Button::new("open-files", "Open files", Tone::Primary).render(palette, window);
        let queue_label = super::queue_count_label(self.core.queue.scheduler().pending_count());
        let queue = Button::new("queue", &queue_label, Tone::Standard).render(palette, window);

        div()
            .id("apollo-root")
            .tab_group()
            .track_focus(&self.focus)
            .size_full()
            .bg(rgba(palette.background))
            .text_color(rgba(palette.foreground))
            .font(components::ui_font())
            .text_size(px(M.body))
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "tab"
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                    && !event.keystroke.modifiers.platform
                {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    cx.stop_propagation();
                }
            })
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_h_full()
                    .p(px(M.inset))
                    .gap(px(M.section))
                    .child(
                        div()
                            .text_size(px(M.title))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Apollo"),
                    )
                    .child(components::separator(palette))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .items_start()
                            .gap(px(M.related))
                            .child(components::empty_state("Unavailable", message, palette)),
                    )
                    .child(components::action_bar(palette, [queue, open])),
            )
    }
}

fn open_root(cx: &mut App) -> Result<(), String> {
    let fonts = cx.text_system();
    fonts
        .add_fonts(vec![Cow::Borrowed(include_bytes!(
            "../../assets/fonts/Geist-Variable.ttf"
        ))])
        .map_err(|error| format!("Could not register Apollo's Geist font: {error:#}"))?;
    if !fonts
        .all_font_names()
        .iter()
        .any(|name| name == FONT_FAMILY)
    {
        return Err("The bundled Geist font family could not be resolved.".into());
    }
    let (width, height) = REFERENCE_CONTENT_DIMENSIONS;
    let (min_width, min_height) = MINIMUM_CONTENT_DIMENSIONS;
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    let mut icon_result = Ok(());
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(min_width), px(min_height))),
            titlebar: Some(TitlebarOptions {
                title: Some("Apollo".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            icon_result = native::set_application_icon(window);
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus);
                ApolloRoot {
                    core: Application::default(),
                    focus,
                }
            })
        },
    )
    .map_err(|error| format!("Could not open Apollo's window: {error:#}"))?;
    icon_result?;
    cx.activate(true);
    Ok(())
}

pub fn run() -> ExitCode {
    let failed = Rc::new(Cell::new(false));
    let startup_failed = Rc::clone(&failed);
    GpuiApplication::new()
        .with_assets(components::Assets)
        .run(move |cx: &mut App| {
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            if let Err(error) = open_root(cx) {
                startup_failed.set(true);
                native::report_startup_error(&error);
                cx.quit();
            }
        });
    if failed.get() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

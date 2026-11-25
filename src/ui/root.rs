//! GPUI application entry and root view. Work remains unavailable until its owners are wired.

use std::{cell::Cell, process::ExitCode, rc::Rc};

use gpui::{
    App, Application as GpuiApplication, Bounds, Context, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};

use crate::app::{Application, NavigationState};
use crate::native;

use super::{MINIMUM_CONTENT_DIMENSIONS, REFERENCE_CONTENT_DIMENSIONS};

struct ApolloRoot {
    core: Application,
}

impl Render for ApolloRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let message = match self.core.navigation() {
            NavigationState::NoSource => "Opening files is unavailable during the UI migration.",
            _ => "This view is unavailable during the UI migration.",
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .bg(rgb(0x000000))
            .text_color(rgb(0xffffff))
            .child(div().text_xl().child("Apollo"))
            .child(div().text_sm().child(message))
    }
}

fn open_root(cx: &mut App) -> Result<(), String> {
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
            cx.new(|_| ApolloRoot {
                core: Application::default(),
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
    GpuiApplication::new().run(move |cx: &mut App| {
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

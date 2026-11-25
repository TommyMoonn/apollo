//! GPUI application root and foreground worker settlement.

use std::{borrow::Cow, cell::Cell, process::ExitCode, rc::Rc};

use gpui::{
    App, Application as GpuiApplication, Bounds, Context, FocusHandle, FontWeight, Task,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgba, size,
};

use crate::app::{Application, NavigationState, background, discovery::MediaToolsState};
use crate::native;
use crate::progress_wake::MediaProgressWakeOwner;

use super::{
    MINIMUM_CONTENT_DIMENSIONS, REFERENCE_CONTENT_DIMENSIONS,
    components::{self, ActionState, Button, Tone},
    design::{FONT_FAMILY, METRICS as M, Palette},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExitState {
    Open,
    Confirming,
    Settling,
    Failed,
}

struct ApolloRoot {
    core: Application,
    focus: FocusHandle,
    dialog_focus: FocusHandle,
    retry_focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    exit: ExitState,
    wake: async_channel::Sender<()>,
    progress_wake: MediaProgressWakeOwner,
    // Dropping the root cancels its subscription and closes the wake receiver.
    _background_task: Task<()>,
}

impl ApolloRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (wake, receiver) = background::wake_channel();
        let progress_sender = wake.clone();
        let progress_wake =
            MediaProgressWakeOwner::new(move || background::notify(&progress_sender));
        let task = cx.spawn_in(window, async move |root, cx| {
            while receiver.recv().await.is_ok() {
                if root.update_in(cx, Self::settle_background).is_err() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut root = Self {
            core: Application::default(),
            focus,
            dialog_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            retry_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            previous_focus: None,
            exit: ExitState::Open,
            wake,
            progress_wake,
            _background_task: task,
        };
        root.retry_discovery();
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            // Defer the decision until GPUI has returned the window to the app.
            let _ = weak.update(cx, |root, cx| {
                if root.exit == ExitState::Open {
                    cx.defer_in(window, Self::request_close);
                }
            });
            false
        });
        root
    }

    fn retry_discovery(&mut self) -> bool {
        if self.exit != ExitState::Open
            || self.core.discovery.is_pending()
            || !matches!(self.core.discovery.state(), MediaToolsState::Unavailable(_))
        {
            return false;
        }
        let sender = self.wake.clone();
        self.core
            .discovery
            .start(move || background::notify(&sender));
        true
    }

    fn settle_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exit == ExitState::Failed {
            return;
        }
        let progress = &mut self.progress_wake;
        if self.core.is_closing() {
            if self.core.poll_shutdown(|| progress.begin()) {
                if self.core.shutdown_failure().is_some() {
                    self.exit = ExitState::Failed;
                    window.focus(&self.dialog_focus);
                    cx.notify();
                } else {
                    window.remove_window();
                }
            }
        } else if self.core.poll_background(|| progress.begin()) {
            cx.notify();
        }
    }

    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exit != ExitState::Open {
            return;
        }
        if self.core.queue.scheduler().pending_count() == 0 {
            self.begin_close(window, cx);
        } else {
            self.previous_focus = window.focused(cx);
            self.exit = ExitState::Confirming;
            window.focus(&self.dialog_focus);
            cx.notify();
        }
    }

    fn dismiss_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exit != ExitState::Confirming {
            return;
        }
        self.exit = ExitState::Open;
        let focus = self
            .previous_focus
            .take()
            .unwrap_or_else(|| self.focus.clone());
        // The retry action may have disappeared while background discovery settled.
        if focus == self.retry_focus
            && !matches!(self.core.discovery.state(), MediaToolsState::Unavailable(_))
        {
            window.focus(&self.focus);
        } else {
            window.focus(&focus);
        }
        cx.notify();
    }

    fn begin_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.core.begin_shutdown() {
            self.exit = ExitState::Settling;
            self.previous_focus = None;
            window.focus(&self.focus);
            cx.notify();
            self.settle_background(window, cx);
        }
    }

    fn discovery_view(&self, palette: Palette, window: &Window, cx: &Context<Self>) -> gpui::Div {
        match self.core.discovery.state() {
            MediaToolsState::Checking => components::pending("Checking media tools", palette),
            MediaToolsState::Ready(_) => components::status("Media tools ready", palette, None),
            MediaToolsState::Unavailable(error) => {
                let mut retry = Button::new("retry-tools", "Retry", Tone::Standard);
                if self.exit == ExitState::Open && !self.core.discovery.is_pending() {
                    retry.state =
                        ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                            if root.retry_discovery() {
                                if root.core.discovery.is_pending() {
                                    window.focus(&root.focus);
                                }
                                cx.notify();
                            }
                        })));
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(M.related))
                    .child(components::status("Media tools unavailable", palette, None))
                    .child(components::inline_message(error.to_string(), palette))
                    .child(
                        div().child(retry.render(palette, window).track_focus(&self.retry_focus)),
                    )
            }
        }
    }

    fn exit_view(&self, palette: Palette, window: &Window, cx: &Context<Self>) -> gpui::Div {
        let content = div().flex().flex_col().gap(px(M.related)).w_full();
        match self.exit {
            ExitState::Confirming => {
                let mut keep = Button::new("keep-working", "Keep working", Tone::Standard);
                keep.state = ActionState::Enabled(Box::new(
                    cx.listener(|root, _, window, cx| root.dismiss_close(window, cx)),
                ));
                let mut exit = Button::new("confirm-exit", "Exit", Tone::Destructive);
                exit.state = ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                    if root.exit == ExitState::Confirming {
                        root.begin_close(window, cx);
                    }
                })));
                content
                    .child(components::empty_state(
                        "Exit Apollo?",
                        "Cancel unfinished jobs and exit? Completed files are kept.",
                        palette,
                    ))
                    .child(components::action_bar(
                        palette,
                        [
                            keep.render(palette, window).track_focus(&self.dialog_focus),
                            exit.render(palette, window),
                        ],
                    ))
            }
            ExitState::Settling => content
                .child(components::empty_state(
                    "Closing Apollo…",
                    "Waiting for background work to settle.",
                    palette,
                ))
                .child(components::pending("Settling background work", palette)),
            ExitState::Failed => {
                let mut exit = Button::new("acknowledge-close", "Exit", Tone::Standard);
                exit.state = ActionState::Enabled(Box::new(cx.listener(|root, _, window, _| {
                    if root.exit == ExitState::Failed {
                        window.remove_window();
                    }
                })));
                content
                    .child(components::empty_state(
                        "A job failed while closing",
                        "Check the diagnostic before exiting. Existing output files are kept.",
                        palette,
                    ))
                    .children(
                        self.core
                            .shutdown_failure()
                            .map(|error| components::inline_message(error.to_string(), palette)),
                    )
                    .child(
                        div().child(exit.render(palette, window).track_focus(&self.dialog_focus)),
                    )
            }
            ExitState::Open => content,
        }
    }
}

impl Render for ApolloRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = match self.core.navigation() {
            NavigationState::NoSource => "Opening files is unavailable during the UI migration.",
            _ => "This view is unavailable during the UI migration.",
        };

        let palette = Palette::DARK;
        let open = Button::new("open-files", "Open files", Tone::Primary).render(palette, window);
        let queue_label = super::queue_count_label(self.core.queue.scheduler().pending_count());
        let queue = Button::new("queue", &queue_label, Tone::Standard).render(palette, window);
        let scheduler = self.core.queue.scheduler();
        let work = scheduler.is_started().then(|| {
            if let Some(fraction) = scheduler
                .active_progress()
                .and_then(|progress| progress.fraction)
            {
                components::progress(fraction, palette)
            } else {
                let label = if scheduler.is_running() {
                    "Processing queue"
                } else {
                    "Waiting to process queue"
                };
                components::pending(label, palette)
            }
        });
        let body = if self.exit == ExitState::Open {
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
                .child(self.discovery_view(palette, window, cx))
                .children(work)
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
                .child(components::action_bar(palette, [queue, open]))
        } else {
            div()
                .flex()
                .flex_col()
                .min_h_full()
                .justify_center()
                .p(px(M.inset))
                .child(self.exit_view(palette, window, cx))
        };

        div()
            .id("apollo-root")
            .tab_group()
            .track_focus(&self.focus)
            .size_full()
            .bg(rgba(palette.background))
            .text_color(rgba(palette.foreground))
            .font(components::ui_font())
            .text_size(px(M.body))
            .on_key_down(cx.listener(|root, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && root.exit == ExitState::Confirming {
                    root.dismiss_close(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "tab"
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
            }))
            .overflow_y_scroll()
            .child(body)
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
            cx.new(|cx| ApolloRoot::new(window, cx))
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

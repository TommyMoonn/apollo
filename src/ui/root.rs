//! GPUI application root and foreground worker settlement.

use std::{borrow::Cow, cell::Cell, path::PathBuf, process::ExitCode, rc::Rc};

use gpui::{
    App, Bounds, Context, FocusHandle, FontWeight, Task, TextRenderingMode, TitlebarOptions,
    Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, div, prelude::*, px, rgba,
    size,
};

use crate::app::{Application, NavigationState, background, discovery::MediaToolsState};
use crate::progress_wake::MediaProgressWakeOwner;
use crate::{
    app::dialogs::{
        DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogRequest, InputDialogOrigin,
    },
    file_dialog::{FileDialogOwner, FileDialogStartError},
    native::{self, WindowOwner},
    settings::{self, AppSettings},
};

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
    open_focus: FocusHandle,
    picker_focus: Option<FocusHandle>,
    file_dialog: FileDialogOwner,
    dialog_origin: Option<InputDialogOrigin>,
    settings_path: Option<PathBuf>,
    settings_error: Option<String>,
    settings_dirty: bool,
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
        window.focus(&focus, cx);
        let (settings_path, preferences, settings_error) = match settings::settings_path() {
            Ok(path) => {
                let (preferences, error) = AppSettings::load(&path);
                (Some(path), preferences, error)
            }
            Err(error) => (None, AppSettings::default(), Some(error)),
        };
        let core = Application::with_settings(preferences);
        let dialog_wake = wake.clone();
        let file_dialog = FileDialogOwner::new(
            move || background::notify(&dialog_wake),
            WindowOwner::from_window(window),
            cx.foreground_executor().clone(),
        );
        let mut root = Self {
            core,
            open_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            picker_focus: None,
            file_dialog,
            dialog_origin: None,
            settings_path,
            settings_error,
            settings_dirty: false,
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

    fn can_open_files(&self) -> bool {
        self.exit == ExitState::Open
            && !self.file_dialog.is_pending()
            && self.core.discovery.ready().is_some()
            && self.core.batch_origin().is_some()
            && !self.core.source.is_inspecting()
            && !self.core.frame_work.copy_active()
    }

    fn start_file_dialog(
        &mut self,
        request: FileDialogRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.exit != ExitState::Open || self.file_dialog.is_pending() {
            return;
        }
        let request = self.core.prepare_file_dialog_request(request);
        let target = request.target();
        let origin = InputDialogOrigin::capture(&self.core);
        match self.file_dialog.request(request) {
            Ok(()) => {
                self.dialog_origin = Some(origin);
                self.picker_focus = window.focused(cx).or_else(|| Some(self.open_focus.clone()));
            }
            Err(FileDialogStartError::AlreadyPending) => return,
            Err(error) => self.apply_dialog(
                origin,
                FileDialogCompletion {
                    target,
                    outcome: DialogOutcome::Failed(error.to_string()),
                },
            ),
        }
        cx.notify();
    }

    fn apply_dialog(&mut self, origin: InputDialogOrigin, completion: FileDialogCompletion) {
        let before = self.core.settings.clone();
        let wake = self.wake.clone();
        self.core
            .settle_native_dialog(origin, completion, move || background::notify(&wake));
        if self.core.settings != before {
            self.settings_dirty = true;
            self.persist_settings();
        }
    }

    fn persist_settings(&mut self) {
        if !self.settings_dirty {
            return;
        }
        if self.settings_path.is_none() {
            match settings::settings_path() {
                Ok(path) => self.settings_path = Some(path),
                Err(error) => {
                    self.settings_error = Some(error);
                    return;
                }
            }
        }
        match self.core.settings.save(
            self.settings_path
                .as_deref()
                .expect("settings path resolved"),
        ) {
            Ok(()) => {
                self.settings_dirty = false;
                self.settings_error = None;
            }
            Err(error) => self.settings_error = Some(error),
        }
    }

    fn poll_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(completion) = self.file_dialog.poll() else {
            return;
        };
        let origin = self.dialog_origin.take();
        if let Some(origin) = origin.filter(|_| !self.core.is_closing()) {
            self.apply_dialog(origin, completion);
        }
        if let Some(focus) = self.picker_focus.take() {
            if self.exit == ExitState::Open {
                // Restore the initiating control when still actionable; otherwise use the root.
                if self.can_open_files() || focus != self.open_focus {
                    window.focus(&focus, cx);
                } else {
                    window.focus(&self.focus, cx);
                }
            } else if self.exit == ExitState::Confirming {
                self.previous_focus = Some(focus);
                window.focus(&self.dialog_focus, cx);
            }
        }
        cx.notify();
    }

    fn settle_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exit == ExitState::Failed {
            return;
        }
        self.poll_file_dialog(window, cx);
        let progress = &mut self.progress_wake;
        if self.core.is_closing() {
            if self.core.poll_shutdown(|| progress.begin()) && !self.file_dialog.is_pending() {
                if self.core.shutdown_failure().is_some() {
                    self.exit = ExitState::Failed;
                    window.focus(&self.dialog_focus, cx);
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
            window.focus(&self.dialog_focus, cx);
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
        if (focus == self.retry_focus
            && !matches!(self.core.discovery.state(), MediaToolsState::Unavailable(_)))
            || (focus == self.open_focus && !self.can_open_files())
        {
            window.focus(&self.focus, cx);
        } else {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn begin_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.core.begin_shutdown() {
            self.exit = ExitState::Settling;
            self.previous_focus = None;
            window.focus(&self.focus, cx);
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
                                    window.focus(&root.focus, cx);
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
            NavigationState::NoSource => "Choose a media file. The source view is being migrated.",
            NavigationState::BatchSetup | NavigationState::BatchEdit => {
                "Batch configuration is unavailable during the UI migration."
            }
            _ => "This view is unavailable during the UI migration.",
        };

        let palette = Palette::DARK;
        let mut open = Button::new("open-files", "Open files", Tone::Primary);
        open.alternate_labels = &["Opening files…"];
        if self.file_dialog.is_pending() {
            open.label = "Opening files…";
            open.state = ActionState::Busy;
        } else if self.can_open_files() {
            open.state = ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                if !root.can_open_files() {
                    return;
                }
                window.focus(&root.open_focus, cx);
                root.start_file_dialog(
                    FileDialogRequest::OpenSources {
                        initial_directory: None,
                        target: DialogTarget::BatchInput,
                    },
                    window,
                    cx,
                );
            })));
        }
        let open = open.render(palette, window);
        let open = if self.can_open_files() {
            open.track_focus(&self.open_focus)
        } else {
            open
        };
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
                .children(
                    self.core
                        .source
                        .is_inspecting()
                        .then(|| components::pending("Checking file…", palette)),
                )
                .children(self.core.source.error().map(|error| {
                    let message = format!(
                        "{} {}",
                        error.message(),
                        error.guidance().unwrap_or_default()
                    );
                    components::inline_message(message, palette)
                }))
                .children(self.settings_error.as_ref().map(|error| {
                    let mut feedback = div()
                        .flex()
                        .flex_col()
                        .gap(px(M.related))
                        .child(components::inline_message(error.clone(), palette));
                    if self.settings_dirty && !self.file_dialog.is_pending() {
                        let mut retry = Button::new(
                            "retry-settings",
                            "Retry saving preferences",
                            Tone::Standard,
                        );
                        retry.state =
                            ActionState::Enabled(Box::new(cx.listener(|root, _, _, cx| {
                                root.persist_settings();
                                cx.notify();
                            })));
                        feedback = feedback.child(retry.render(palette, window));
                    }
                    feedback
                }))
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
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
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
            // Subpixel glyph rendering requires an opaque native window.
            window_background: WindowBackgroundAppearance::Opaque,
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
    gpui_platform::application()
        .with_assets(components::Assets)
        .run(move |cx: &mut App| {
            // Respect the OS text-smoothing preference instead of forcing ClearType.
            cx.set_text_rendering_mode(TextRenderingMode::PlatformDefault);
            cx.on_window_closed(|cx, _window_id| {
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

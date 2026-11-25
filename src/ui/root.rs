//! GPUI application root and foreground worker settlement.

use std::{
    borrow::Cow,
    cell::Cell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::ExitCode,
    rc::Rc,
};

use gpui::{
    App, Bounds, Context, ExternalPaths, FocusHandle, Task, TextRenderingMode, TitlebarOptions,
    Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, div, prelude::*, px, rgba,
    size,
};

use crate::app::{
    Application, NavigationState, Operation, QueueAction, background, discovery::MediaToolsState,
    queue::QueueEffect,
};
use crate::media::container::MediaContainer;
use crate::progress_wake::MediaProgressWakeOwner;
use crate::queue::JobId;
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
    components::{self, ActionState, Button, ProgressState, Tone},
    design::{FONT_FAMILY, METRICS as M, Palette, TextRole, WORDMARK_FAMILY},
    file_intake,
    queue::{self, FinishedAction, WaitingAction, WaitingControls},
    shell,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExitState {
    Open,
    Confirming,
    Settling,
    Failed,
}

#[derive(Clone, Copy)]
enum DropContext {
    Eligible(InputDialogOrigin),
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LandingContent {
    Discovering,
    ToolsUnavailable,
    Inspecting,
    Ready,
}

fn landing_content(core: &Application) -> LandingContent {
    if core.source.is_inspecting() {
        return LandingContent::Inspecting;
    }
    match core.discovery.state() {
        MediaToolsState::Checking => LandingContent::Discovering,
        MediaToolsState::Unavailable(_) => LandingContent::ToolsUnavailable,
        MediaToolsState::Ready(_) => LandingContent::Ready,
    }
}

/// The active job owns progress through cancellation settlement, even after the queue is stopped.
fn queue_progress_state(
    run_state: crate::queue::QueueRunState,
    active: bool,
    cancelling: bool,
    fraction: Option<f32>,
) -> ProgressState<'static> {
    if active || cancelling {
        ProgressState::Active {
            label: if cancelling {
                "Cancelling queue job"
            } else if fraction.is_some_and(|value| value.is_finite() && value >= 1.) {
                "Finalizing queue job"
            } else {
                "Processing queue"
            },
            fraction: queue::unsettled_fraction(fraction),
        }
    } else if run_state != crate::queue::QueueRunState::Stopped {
        ProgressState::Waiting("Waiting to process queue")
    } else {
        ProgressState::Idle
    }
}

const REMUX_FORMATS: [MediaContainer; 2] = [MediaContainer::Mp4, MediaContainer::Matroska];

struct RemuxControls {
    session: u64,
    input: gpui::Entity<super::text_input::TextInput>,
    format: gpui::Entity<super::dropdown::Dropdown>,
    _format_subscription: gpui::Subscription,
    _subscription: gpui::Subscription,
    _observation: gpui::Subscription,
}

struct MixerTrackControl {
    presentation: super::audio_mix::TrackPresentation,
    metadata: String,
    gain: gpui::Entity<super::text_input::TextInput>,
    _subscription: gpui::Subscription,
}
struct MixerControls {
    session: u64,
    tracks: Vec<MixerTrackControl>,
}
struct ApolloRoot {
    core: Application,
    focus: FocusHandle,
    whole_page_scroll: Rc<Cell<bool>>,
    dialog_focus: FocusHandle,
    retry_focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    queue_cancel_confirming: bool,
    queue_cancel_focus: FocusHandle,
    queue_back_focus: FocusHandle,
    waiting_focus: HashMap<(JobId, WaitingAction), FocusHandle>,
    finished_focus: HashMap<(JobId, FinishedAction), FocusHandle>,
    expanded_failure: Option<JobId>,
    finished_error: Option<(JobId, String)>,
    open_focus: FocusHandle,
    picker_focus: Option<FocusHandle>,
    drag_origin: Option<DropContext>,
    remux_controls: Option<RemuxControls>,
    mixer_controls: Option<MixerControls>,
    browse_focus: FocusHandle,
    browse_pending: bool,
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
            drag_origin: None,
            remux_controls: None,
            mixer_controls: None,
            browse_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            browse_pending: false,
            file_dialog,
            dialog_origin: None,
            settings_path,
            settings_error,
            settings_dirty: false,
            focus,
            whole_page_scroll: Rc::new(Cell::new(true)),
            dialog_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            retry_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            previous_focus: None,
            queue_cancel_confirming: false,
            queue_cancel_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            queue_back_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            waiting_focus: HashMap::new(),
            finished_focus: HashMap::new(),
            expanded_failure: None,
            finished_error: None,
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
            && !self.queue_cancel_confirming
            && !self.file_dialog.is_pending()
            && matches!(
                self.core.navigation(),
                NavigationState::NoSource | NavigationState::SourceWorkspace { .. }
            )
            && self.core.can_accept_source_paths()
    }

    fn can_add_queue_files(&self) -> bool {
        self.can_navigate()
            && self.core.queue_origin().is_some()
            && self.core.can_accept_source_paths()
    }

    fn add_queue_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_add_queue_files() {
            return;
        }
        let progress = &mut self.progress_wake;
        if matches!(
            self.core
                .queue_action(crate::app::QueueAction::AddFiles, || progress.begin()),
            crate::app::queue::QueueEffect::AddFiles
        ) {
            window.focus(&self.open_focus, cx);
            self.start_file_dialog(
                FileDialogRequest::OpenSources {
                    initial_directory: None,
                    target: DialogTarget::BatchInput,
                },
                window,
                cx,
            );
        }
    }

    fn queue_controls(&self) -> queue::Controls {
        queue::Controls::for_app(&self.core)
    }

    fn dispatch_queue_action(
        &mut self,
        action: QueueAction,
        expected_job: Option<JobId>,
        cx: &mut Context<Self>,
    ) {
        if !self.can_navigate() || self.core.queue_origin().is_none() {
            return;
        }
        if !self.queue_controls().allows(action, expected_job) {
            self.core.queue_action_error =
                Some("The queue has changed. Check its current state before trying again.".into());
            cx.notify();
            return;
        }
        let progress = &mut self.progress_wake;
        if matches!(
            self.core.queue_action(action, || progress.begin()),
            crate::app::queue::QueueEffect::Handled(false)
        ) {
            self.core.queue_action_error.get_or_insert_with(|| "The action could not be completed. Check the current queue state and try again if available.".into());
        }
        cx.notify();
    }

    fn dispatch_waiting_action(
        &mut self,
        id: JobId,
        action: WaitingAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_navigate() || self.core.queue_origin().is_none() {
            return;
        }
        let Some(controls) = WaitingControls::for_job(self.core.queue.scheduler(), id) else {
            self.core.queue_action_error = Some("This job is no longer waiting.".into());
            window.focus(&self.queue_back_focus, cx);
            cx.notify();
            return;
        };
        if !controls.allows(action) {
            self.core.queue_action_error = Some(
                controls
                    .edit_blocked
                    .filter(|_| action == WaitingAction::Edit)
                    .unwrap_or("The waiting order has changed. Choose an available move.")
                    .into(),
            );
            self.focus_waiting_after(id, controls.index, action, window, cx);
            cx.notify();
            return;
        }
        let progress = &mut self.progress_wake;
        match self
            .core
            .queue_action(action.command(id), || progress.begin())
        {
            QueueEffect::Handled(true) => {
                self.focus_waiting_after(id, controls.index, action, window, cx)
            }
            QueueEffect::Edit(id, origin) => {
                if let Some(edit) = self.core.prepare_queue_edit(id, origin) {
                    self.core.activate_queue_edit(edit, origin);
                    self.invalidate_drag();
                    window.focus(&self.focus, cx);
                } else {
                    self.core.queue_action_error = Some("The job can no longer be edited.".into());
                }
            }
            _ => {
                self.core.queue_action_error.get_or_insert_with(|| {
                    "The waiting job could not be changed. Check the queue and try again.".into()
                });
            }
        }
        cx.notify();
    }

    fn dispatch_finished_action(
        &mut self,
        id: JobId,
        action: FinishedAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_navigate() || self.core.queue_origin().is_none() {
            return;
        }
        let Some(job) = self
            .core
            .queue
            .scheduler()
            .jobs()
            .iter()
            .find(|job| job.id() == id)
        else {
            window.focus(&self.queue_back_focus, cx);
            cx.notify();
            return;
        };
        if !action.allows(job)
            || (action == FinishedAction::CopyDetails && self.expanded_failure != Some(id))
        {
            return;
        }
        if let Some(focus) = self.finished_focus.get(&(id, action)) {
            window.focus(focus, cx);
        }
        match action {
            FinishedAction::Details => {
                self.expanded_failure = (self.expanded_failure != Some(id)).then_some(id);
            }
            FinishedAction::CopyDetails => {
                if let Some(details) = queue::failure_details(job) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(details));
                }
            }
            _ => {
                let Some(command) = action.command(id) else {
                    return;
                };
                self.finished_error = None;
                let progress = &mut self.progress_wake;
                match self.core.queue_action(command, || progress.begin()) {
                    QueueEffect::Handled(true) => {}
                    QueueEffect::Output(action, output) => {
                        self.core
                            .settle_output_action(action, &output, action.run(&output));
                    }
                    _ => {
                        self.core.queue_action_error.get_or_insert_with(|| "The job action is no longer available. Check the queue and try again.".into());
                    }
                }
                self.finished_error = self.core.queue_action_error.take().map(|error| (id, error));
            }
        }
        cx.notify();
    }

    fn clear_finished(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_navigate() || self.core.queue_origin().is_none() {
            return;
        }
        let progress = &mut self.progress_wake;
        if matches!(
            self.core
                .queue_action(QueueAction::ClearFinished, || progress.begin()),
            QueueEffect::Handled(true)
        ) {
            self.expanded_failure = None;
            self.finished_error = None;
            self.finished_focus.clear();
            window.focus(&self.queue_back_focus, cx);
        }
        cx.notify();
    }

    fn focus_waiting_after(
        &self,
        id: JobId,
        previous_index: usize,
        action: WaitingAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target =
            queue::waiting_focus_after(self.core.queue.scheduler(), id, previous_index, action);
        let focus = target
            .and_then(|target| self.waiting_focus.get(&target))
            .unwrap_or(&self.queue_back_focus);
        window.focus(focus, cx);
    }

    fn finish_queue_edit(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_navigate()
            || !matches!(self.core.navigation(), NavigationState::QueueEdit { .. })
        {
            return;
        }
        let Some(edit) = self.core.queue_edit() else {
            return;
        };
        let id = edit.job_id;
        if save && (!queue::editor_migrated(edit.active_operation) || !self.core.save_queue_edit())
        {
            cx.notify();
            return;
        }
        if self.core.finish_queue_edit().is_some() {
            self.invalidate_drag();
            if let Some(controls) = WaitingControls::for_job(self.core.queue.scheduler(), id) {
                self.focus_waiting_after(id, controls.index, WaitingAction::Edit, window, cx);
            } else {
                window.focus(&self.queue_back_focus, cx);
            }
        }
        cx.notify();
    }

    fn request_cancel_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_navigate()
            || self.core.queue_origin().is_none()
            || !self.queue_controls().cancel_queue
        {
            return;
        }
        self.invalidate_drag();
        self.previous_focus = window.focused(cx);
        self.queue_cancel_confirming = true;
        window.focus(&self.dialog_focus, cx);
        cx.notify();
    }

    fn resolve_cancel_queue(
        &mut self,
        confirmed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !std::mem::take(&mut self.queue_cancel_confirming) {
            return;
        }
        if confirmed {
            self.dispatch_queue_action(QueueAction::CancelQueue, None, cx);
        }
        let previous = self.previous_focus.take();
        let focus = if self.queue_controls().cancel_queue && self.can_navigate() {
            previous.unwrap_or_else(|| self.queue_cancel_focus.clone())
        } else {
            self.focus.clone()
        };
        window.focus(&focus, cx);
        cx.notify();
    }

    fn cancel_queue_view(
        &self,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let mut keep = Button::new("keep-queue", "Keep queue", Tone::Standard);
        keep.state = ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
            root.resolve_cancel_queue(false, window, cx);
        })));
        let mut cancel = Button::new("confirm-cancel-queue", "Cancel queue", Tone::Destructive);
        if self.queue_controls().cancel_queue {
            cancel.state = ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                root.resolve_cancel_queue(true, window, cx);
            })));
        }
        components::confirmation(
            "Cancel queue?",
            "Cancel unfinished jobs and stop the queue? Completed files are kept.",
            [
                keep.render(palette, window).track_focus(&self.dialog_focus),
                cancel.render(palette, window),
            ],
            palette,
        )
    }

    fn accept_drop(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        let Some(DropContext::Eligible(origin)) = self.drag_origin.take() else {
            return;
        };
        if !self.can_open_files() || !origin.is_current(&self.core) {
            return;
        }
        let before = self.core.settings.clone();
        let wake = self.wake.clone();
        self.core
            .accept_source_paths(origin, paths.paths().to_vec(), move || {
                background::notify(&wake)
            });
        self.persist_changed_settings(&before);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn close_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exit != ExitState::Open
            || self.file_dialog.is_pending()
            || !matches!(
                self.core.navigation(),
                NavigationState::SourceWorkspace { .. }
            )
        {
            return;
        }
        self.core.close_source();
        self.invalidate_drag();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn can_navigate(&self) -> bool {
        self.exit == ExitState::Open
            && !self.queue_cancel_confirming
            && !self.file_dialog.is_pending()
    }

    fn navigate(
        &mut self,
        enter: fn(&mut Application) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_navigate() {
            return;
        }
        if enter(&mut self.core) {
            self.invalidate_drag();
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_navigate() {
            return;
        }
        match self.core.navigation() {
            NavigationState::Queue { .. } => self.core.return_from_queue(),
            NavigationState::Settings { .. } => self.core.return_from_settings(),
            NavigationState::QueueEdit { .. } => {
                self.finish_queue_edit(false, window, cx);
                return;
            }
            NavigationState::BatchSetup => {
                self.core.finish_batch_setup();
            }
            _ => return,
        }
        self.invalidate_drag();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn select_operation(&mut self, operation: Operation, cx: &mut Context<Self>) {
        if self.can_navigate() && self.core.select_operation(operation) {
            self.invalidate_drag();
            // The selector keeps its stable element ID and focus across operation switches.
            cx.notify();
        }
    }

    fn invalidate_drag(&mut self) {
        if self.drag_origin.is_some() {
            self.drag_origin = Some(DropContext::Blocked);
        }
    }

    fn mixer_tracks(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        use super::{
            audio_mix::{AudioMixPresentation, parse_track_gain},
            text_input::{Changed, TextInput},
        };
        use crate::app::audio_mix::TrackAction;
        let Some((session, media, draft)) = self.core.audio_mix_context() else {
            return components::inline_message("The source is no longer available.", palette);
        };
        let enabled = self.can_navigate() && !self.core.source.is_inspecting();
        if self
            .mixer_controls
            .as_ref()
            .is_none_or(|controls| controls.session != session)
        {
            let tracks = AudioMixPresentation::new(media)
                .tracks
                .into_iter()
                .map(|presentation| {
                    let id = presentation.stream_index;
                    let label = format!("{} (stream {id}) gain", presentation.title);
                    let gain = cx.new(|cx| {
                        TextInput::new_gain(
                            draft.track_gain(id).unwrap_or_default(),
                            label,
                            window,
                            cx,
                        )
                    });
                    let subscription = cx.subscribe(&gain, move |root, _, change: &Changed, cx| {
                        if root.can_navigate()
                            && let Some(value) = parse_track_gain(&change.0)
                            && root
                                .core
                                .edit_audio_mix_tracks(session, TrackAction::Gain(id, value))
                        {
                            cx.notify();
                        }
                    });
                    let metadata = match &presentation.metadata {
                        Some(metadata) => format!("Stream {id} · {metadata}"),
                        None => format!("Stream {id}"),
                    };
                    MixerTrackControl {
                        presentation,
                        metadata,
                        gain,
                        _subscription: subscription,
                    }
                })
                .collect();
            self.mixer_controls = Some(MixerControls { session, tracks });
        }
        let controls = self
            .mixer_controls
            .as_ref()
            .expect("controls for current source");
        let action = |id, label, action, allowed: bool| {
            let mut button = Button::new(id, label, Tone::Quiet);
            if enabled && allowed {
                button.state =
                    ActionState::Enabled(Box::new(cx.listener(move |root, _, _, cx| {
                        if root.can_navigate() && root.core.edit_audio_mix_tracks(session, action) {
                            cx.notify();
                        }
                    })));
            }
            button.render(palette, window)
        };
        let selected_count = draft.selected_count();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap(px(M.compact))
            .child(components::heading("Audio tracks"))
            .child(
                div()
                    .flex()
                    .gap(px(M.compact))
                    .child(action(
                        "select-all-tracks",
                        "Select all",
                        TrackAction::SelectAll,
                        selected_count < controls.tracks.len(),
                    ))
                    .child(action(
                        "clear-tracks",
                        "Clear",
                        TrackAction::Clear,
                        selected_count > 0,
                    )),
            );
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap(px(M.compact))
            .child(
                TextRole::Secondary
                    .apply(div())
                    .text_color(rgba(palette.secondary()))
                    .child(format!("{selected_count} selected")),
            )
            .child(action(
                "reset-track-gains",
                "Reset gains",
                TrackAction::ResetGains,
                !controls.tracks.is_empty(),
            ));
        let narrow = window.viewport_size().width < px(480.);
        let rows = controls
            .tracks
            .iter()
            .map(|track| {
                let info = &track.presentation;
                let id = info.stream_index;
                let selected = draft.is_selected(id);
                track.gain.update(cx, |field, _| {
                    field.sync_gain(
                        draft.track_gain(id).unwrap_or_default(),
                        enabled,
                        palette,
                        window,
                    )
                });
                let metadata = track.metadata.clone();
                let title = info.title.clone();
                let accessible = format!("{title}, {metadata}");
                let marker = div()
                    .size(px(16.))
                    .flex_shrink_0()
                    .rounded(px(M.radius))
                    .border_1()
                    .border_color(rgba(if selected {
                        palette.foreground
                    } else {
                        palette.border()
                    }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |marker| {
                        marker
                            .bg(rgba(palette.foreground))
                            .text_color(rgba(palette.background))
                            .child("✓")
                    });
                let identity = div()
                    .id(("select-track", id as usize))
                    .role(gpui::Role::CheckBox)
                    .aria_label(accessible.clone())
                    .aria_toggled(if selected {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .flex()
                    .items_center()
                    .gap(px(M.compact))
                    .min_w_0()
                    .flex_1()
                    .rounded(px(M.radius))
                    .border_1()
                    .border_color(rgba(0))
                    .p(px(M.tight))
                    .when(narrow, |row| row.w_full())
                    .when(enabled, |row| {
                        row.tab_index(0)
                            .cursor_pointer()
                            .focus_visible(move |style| {
                                style.border_color(rgba(palette.foreground))
                            })
                            .on_click(cx.listener(move |root, _, _, cx| {
                                if root.can_navigate()
                                    && root
                                        .core
                                        .edit_audio_mix_tracks(session, TrackAction::Toggle(id))
                                {
                                    cx.notify();
                                }
                            }))
                    })
                    .child(marker)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .flex_1()
                            .child(TextRole::Body.apply(div()).truncate().child(title))
                            .child(
                                TextRole::Secondary
                                    .apply(div())
                                    .text_color(rgba(palette.secondary()))
                                    .truncate()
                                    .child(metadata),
                            ),
                    )
                    .tooltip(components::tooltip(accessible, palette));
                let gain = div()
                    .flex()
                    .items_center()
                    .gap(px(M.tight))
                    .w(px(super::theme::TRACK_GAIN_CONTROL_WIDTH))
                    .flex_shrink_0()
                    .child(div().flex_1().min_w_0().child(track.gain.clone()))
                    .child(TextRole::Secondary.apply(div()).flex_shrink_0().child("dB"));
                div()
                    .id(("audio-track", id as usize))
                    .flex()
                    .gap(px(M.compact))
                    .p(px(M.tight))
                    .rounded(px(M.radius))
                    .when(selected, |row| row.bg(rgba(palette.hover())))
                    .when(enabled, |row| {
                        row.hover(move |style| style.bg(rgba(palette.hover())))
                    })
                    .when(narrow, |row| row.flex_col().items_start())
                    .when(!narrow, |row| row.items_center())
                    .child(identity)
                    .child(gain)
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap(px(M.compact))
            .w_full()
            .min_w_0()
            .child(header)
            .child(components::separator(palette))
            .when(rows.is_empty(), |body| {
                body.child(components::inline_message("No audio tracks.", palette))
            })
            .children(rows)
            .child(components::separator(palette))
            .child(footer)
    }

    fn remux_form(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (gpui::Div, gpui::Div) {
        use super::{
            dropdown::{Dropdown, Selected},
            output,
            remux::{RemuxPresentation, remux_ready_feedback},
            text_input::{Changed, TextInput},
        };
        use crate::app::{dialogs::OutputDraft, queue::QueueSubmission};
        let Some((session, media, draft)) = self.core.remux_context() else {
            return (
                components::inline_message("The source is no longer available.", palette),
                div(),
            );
        };
        let editing = matches!(self.core.navigation(), NavigationState::QueueEdit { .. });
        let validation = draft.validation(media);
        let feedback = validation.as_ref().err().map(|error| {
            remux_ready_feedback(error, &RemuxPresentation::new(media))
                .unwrap_or_else(|| error.to_string())
        });
        let current_container = draft.container();
        let format_index = usize::from(current_container == MediaContainer::Matroska);
        let directory = draft.output_directory().to_path_buf();
        let stem = draft.output_filename_stem().to_string_lossy().into_owned();
        let editable_name = draft.output_filename_stem().to_str().is_some();
        let enabled = self.can_navigate()
            && !self.core.source.is_inspecting()
            && self.core.discovery.ready().is_some();
        if self
            .remux_controls
            .as_ref()
            .is_none_or(|field| field.session != session)
        {
            let input = cx.new(|cx| TextInput::new(stem.clone(), cx));
            let subscription = cx.subscribe(&input, move |root, _, change: &Changed, cx| {
                if root.can_navigate()
                    && let Some(draft) = root.core.remux_draft_mut(session)
                {
                    draft.set_output_filename_stem(change.0.clone().into());
                    cx.notify();
                }
            });
            // Composition can change action readiness even before the typed value changes.
            let observation = cx.observe(&input, |_, _, cx| cx.notify());
            let format = cx.new(|cx| Dropdown::new(&["MP4", "MKV"], format_index, cx));
            let format_subscription =
                cx.subscribe(&format, move |root, _, change: &Selected, cx| {
                    if root.can_navigate()
                        && !root.remux_composing(session, cx)
                        && let Some(&container) = REMUX_FORMATS.get(change.0)
                        && let Some(draft) = root.core.remux_draft_mut(session)
                    {
                        draft.set_container(container);
                        cx.notify();
                    }
                });
            self.remux_controls = Some(RemuxControls {
                session,
                input,
                format,
                _format_subscription: format_subscription,
                _subscription: subscription,
                _observation: observation,
            });
        }
        let input = self
            .remux_controls
            .as_ref()
            .expect("filename for current session")
            .input
            .clone();
        input.update(cx, |input, _| {
            input.sync(&stem, enabled && editable_name, palette)
        });
        let composing = input.read(cx).composing();
        let actions_enabled = enabled && !composing;
        let format = self
            .remux_controls
            .as_ref()
            .expect("controls for current session")
            .format
            .clone();
        format.update(cx, |format, cx| {
            format.sync(format_index, actions_enabled, palette, cx)
        });
        let mut browse = Button::new("browse-output", "Browse", Tone::Standard);
        browse.alternate_labels = &["Browse"];
        if self.file_dialog.is_pending() && self.browse_pending {
            browse.state = ActionState::Busy;
        } else if actions_enabled {
            browse.state =
                ActionState::Enabled(Box::new(cx.listener(move |root, event, window, cx| {
                    if !root.can_navigate() || root.core.source.is_inspecting() {
                        return;
                    }
                    let Some((id, _, draft)) = root.core.remux_context() else {
                        return;
                    };
                    if id != session || root.remux_composing(session, cx) {
                        return;
                    }
                    let request = OutputDraft::Remux(draft).request(id);
                    // Keep keyboard traversal intact, but do not restore a mouse
                    // activation to a focused Browse button after the native dialog.
                    let focus = if matches!(event, gpui::ClickEvent::Keyboard(_)) {
                        &root.browse_focus
                    } else {
                        &root.focus
                    };
                    window.focus(focus, cx);
                    root.start_file_dialog(request, window, cx);
                })));
        }
        let browse_enabled = matches!(&browse.state, ActionState::Enabled(_));
        let browse = browse
            .render(palette, window)
            .when(browse_enabled, |button| {
                button.track_focus(&self.browse_focus)
            });
        let error = if editing {
            self.core.queue_edit().and_then(|edit| edit.error.as_ref())
        } else {
            self.core
                .operation_action_error
                .as_ref()
                .filter(|(operation, _)| *operation == Operation::Remux)
                .map(|(_, error)| error)
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(M.related))
            .w_full()
            .min_w_0()
            .child(
                output::OutputRow {
                    format: div().flex_shrink_0().child(format),
                    filename: input,
                    extension: current_container.extension(),
                    extensions: &REMUX_FORMATS.map(MediaContainer::extension),
                    directory: &directory,
                    browse,
                }
                .render(palette, window),
            )
            .children((!editable_name).then(|| {
                components::inline_message(
                    "This filename cannot be represented as text. Use Browse to choose a new name.",
                    palette,
                )
            }))
            .children(feedback.map(|text| components::inline_message(text, palette)))
            .children(error.map(|error| {
                components::inline_message(
                    format!(
                        "{} {} {}",
                        error.message(),
                        error.guidance().unwrap_or_default(),
                        error.diagnostic().unwrap_or_default()
                    ),
                    palette,
                )
            }));
        let ready = actions_enabled && validation.is_ok();
        let mut actions = Vec::new();
        if editing {
            let mut cancel = Button::new(
                "cancel-queue-edit",
                super::CANCEL_QUEUE_EDIT_LABEL,
                Tone::Standard,
            );
            if self.can_navigate() {
                cancel.state =
                    ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                        root.finish_queue_edit(false, window, cx)
                    })));
            }
            let mut save = Button::new(
                "save-queue-edit",
                super::SAVE_QUEUE_CHANGES_LABEL,
                Tone::Primary,
            );
            if ready && self.core.queue.scheduler().can_edit() {
                save.state =
                    ActionState::Enabled(Box::new(cx.listener(move |root, _, window, cx| {
                        if root.core.remux_context().map(|context| context.0) == Some(session)
                            && !root.remux_composing(session, cx)
                        {
                            root.finish_queue_edit(true, window, cx);
                        }
                    })));
            }
            actions.extend([cancel.render(palette, window), save.render(palette, window)]);
        } else {
            let process_now = self.core.process_now_allowed();
            let mut add = Button::new(
                "add-to-queue",
                if self.core.queue_confirmation == Some(Operation::Remux) {
                    super::ADDED_TO_QUEUE_LABEL
                } else {
                    super::ADD_TO_QUEUE_LABEL
                },
                if process_now {
                    Tone::Standard
                } else {
                    Tone::Primary
                },
            );
            add.alternate_labels = &[super::ADD_TO_QUEUE_LABEL, super::ADDED_TO_QUEUE_LABEL];
            if ready {
                add.state =
                    ActionState::Enabled(Box::new(cx.listener(move |root, _, window, cx| {
                        root.submit_remux(session, QueueSubmission::AddToQueue, window, cx)
                    })));
            }
            if process_now {
                let mut process =
                    Button::new("process-now", super::PROCESS_NOW_LABEL, Tone::Primary);
                if ready {
                    process.state =
                        ActionState::Enabled(Box::new(cx.listener(move |root, _, window, cx| {
                            root.submit_remux(session, QueueSubmission::ProcessNow, window, cx)
                        })));
                }
                actions.push(process.render(palette, window));
            }
            actions.push(add.render(palette, window));
        }
        (body, components::action_bar(palette, actions))
    }

    fn remux_composing(&self, session: u64, cx: &App) -> bool {
        self.remux_controls
            .as_ref()
            .is_some_and(|field| field.session == session && field.input.read(cx).composing())
    }

    fn submit_remux(
        &mut self,
        session: u64,
        submission: crate::app::queue::QueueSubmission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_navigate() || self.remux_composing(session, cx) {
            return;
        }
        let progress = &mut self.progress_wake;
        if self
            .core
            .submit_remux(session, submission, || progress.begin())
            && submission == crate::app::queue::QueueSubmission::ProcessNow
        {
            self.invalidate_drag();
            window.focus(&self.queue_back_focus, cx);
        }
        cx.notify();
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
        self.invalidate_drag();
        let request = self.core.prepare_file_dialog_request(request);
        let target = request.target();
        let origin = InputDialogOrigin::capture(&self.core);
        match self.file_dialog.request(request) {
            Ok(()) => {
                self.dialog_origin = Some(origin);
                self.browse_pending = target.output().is_some();
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
        self.persist_changed_settings(&before);
    }

    fn persist_changed_settings(&mut self, before: &AppSettings) {
        if &self.core.settings != before {
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
        self.browse_pending = false;
        let origin = self.dialog_origin.take();
        if let Some(origin) = origin.filter(|_| !self.core.is_closing()) {
            self.apply_dialog(origin, completion);
        }
        if let Some(focus) = self.picker_focus.take() {
            if self.exit == ExitState::Open {
                // Restore the initiating control when still actionable; otherwise use the root.
                if self.can_open_files() || self.can_add_queue_files() || focus != self.open_focus {
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
            if self.can_navigate()
                && self.core.queue_origin().is_some()
                && let Some(focused) = window.focused(cx)
                && let Some((&(id, action), _)) = self
                    .waiting_focus
                    .iter()
                    .find(|(_, handle)| **handle == focused)
            {
                match WaitingControls::for_job(self.core.queue.scheduler(), id) {
                    Some(controls) if !controls.allows(action) => {
                        self.focus_waiting_after(id, controls.index, action, window, cx)
                    }
                    None => window.focus(&self.queue_back_focus, cx),
                    _ => {}
                }
            }
            cx.notify();
        }
    }

    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exit != ExitState::Open {
            return;
        }
        if self.queue_cancel_confirming {
            self.resolve_cancel_queue(false, window, cx);
        }
        self.invalidate_drag();
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
            || (focus == self.open_focus && !self.can_open_files() && !self.can_add_queue_files())
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
            MediaToolsState::Checking => components::centered_spinner(
                "discovery",
                "Checking media tools",
                palette,
                !cx.reduce_motion(),
            ),
            MediaToolsState::Ready(_) => div(),
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
                    .w_full()
                    .min_w_0()
                    .gap(px(M.related))
                    .child(components::status("Media tools unavailable", palette, None))
                    .child({
                        let error = crate::media::error::MediaError::from_tools_error(error);
                        components::inline_message(
                            format!(
                                "{} {}",
                                error.message(),
                                error.guidance().unwrap_or_default()
                            ),
                            palette,
                        )
                    })
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
                components::confirmation(
                    "Exit Apollo?",
                    "Cancel unfinished jobs and exit? Completed files are kept.",
                    [
                        keep.render(palette, window).track_focus(&self.dialog_focus),
                        exit.render(palette, window),
                    ],
                    palette,
                )
            }
            ExitState::Settling => components::centered_spinner(
                "shutdown",
                "Closing Apollo",
                palette,
                !cx.reduce_motion(),
            ),
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
                    .children(self.core.shutdown_failure().map(|error| {
                        TextRole::Secondary
                            .apply(div())
                            .text_color(rgba(palette.secondary()))
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .child(error.to_string())
                    }))
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
        let palette = Palette::DARK;
        if self.remux_controls.as_ref().is_some_and(|field| {
            self.core.remux_context().map(|context| context.0) != Some(field.session)
        }) {
            self.remux_controls = None;
        }
        if self.mixer_controls.as_ref().is_some_and(|controls| {
            self.core.audio_mix_context().map(|context| context.0) != Some(controls.session)
        }) {
            self.mixer_controls = None;
        }
        if !cx.has_active_drag() {
            self.drag_origin = None;
        }
        let has_source = matches!(
            self.core.navigation(),
            NavigationState::SourceWorkspace { .. }
        );
        let inspecting = self.core.source.is_inspecting();
        let mut open = Button::new(
            "open-files",
            if has_source {
                "Change file"
            } else {
                "Open files"
            },
            Tone::Standard,
        );
        open.alternate_labels = &["Open files", "Change file"];
        if (self.file_dialog.is_pending() && !self.browse_pending) || (has_source && inspecting) {
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
        let mut queue = Button::new("queue", &queue_label, Tone::Standard);
        queue.alternate_labels = &["Queue", "Queue (99)"];
        let mut settings = Button::new("settings", "Settings", Tone::Standard);
        if self.can_navigate() {
            queue.state = ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                root.navigate(Application::enter_queue, window, cx)
            })));
            settings.state = ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                root.navigate(Application::enter_settings, window, cx)
            })));
        }
        let mut back = Button::new("shell-back", "Back", Tone::Standard);
        if self.can_navigate() {
            back.state = ActionState::Enabled(Box::new(
                cx.listener(|root, _, window, cx| root.back(window, cx)),
            ));
        }
        let route = self.core.navigation();
        let landing = route == NavigationState::NoSource;
        let animate = !cx.reduce_motion();
        let (header, surface, footer) = match route {
            NavigationState::NoSource => (
                div().flex().justify_center().child(file_intake::wordmark()),
                match landing_content(&self.core) {
                    LandingContent::Inspecting => components::centered_spinner(
                        "source-inspection",
                        "Checking file",
                        palette,
                        animate,
                    ),
                    LandingContent::Discovering | LandingContent::ToolsUnavailable => {
                        self.discovery_view(palette, window, cx)
                    }
                    LandingContent::Ready => file_intake::landing(open, palette),
                },
                shell::navigation_actions(queue, settings, palette, window),
            ),
            NavigationState::SourceWorkspace { active_operation } => {
                let mut close = Button::new("close-source", "Back to file selection", Tone::Quiet);
                if self.exit == ExitState::Open && !self.file_dialog.is_pending() {
                    close.state =
                        ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                            root.close_source(window, cx);
                        })));
                }
                let close = close.render_icon(components::ActionIcon::Back, palette, window);
                let selector = components::selector(Operation::ALL.map(|operation| {
                    let mut button =
                        Button::new(operation.label(), operation.label(), Tone::Standard);
                    if self.can_navigate() {
                        let select =
                            Box::new(cx.listener(move |root, _, _, cx| {
                                root.select_operation(operation, cx)
                            }));
                        button.state = if operation == active_operation {
                            ActionState::Selected(select)
                        } else {
                            ActionState::Enabled(select)
                        };
                    }
                    button.render(palette, window)
                }));
                let header = div()
                    .flex()
                    .flex_col()
                    .gap(px(M.related))
                    .children(self.core.source.current().map(|media| {
                        file_intake::summary(
                            super::workspace::WorkspacePresentation::new(media),
                            Some(close),
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_end()
                                .gap(px(M.action_gap))
                                .child(open)
                                .child(shell::navigation_actions(queue, settings, palette, window)),
                            palette,
                        )
                    }))
                    .child(selector)
                    .child(components::separator(palette));
                if active_operation == Operation::Remux && self.core.discovery.ready().is_some() {
                    let (body, actions) = self.remux_form(palette, window, cx);
                    (header, body, actions)
                } else if active_operation == Operation::AudioMix
                    && self.core.discovery.ready().is_some()
                {
                    (header, self.mixer_tracks(palette, window, cx), div())
                } else {
                    let surface = if self.core.discovery.ready().is_none() {
                        self.discovery_view(palette, window, cx)
                    } else {
                        components::empty_state(
                            active_operation.label(),
                            "This operation is unavailable during the UI migration.",
                            palette,
                        )
                    };
                    (header, surface, div())
                }
            }
            NavigationState::BatchSetup => {
                let count = self
                    .core
                    .batch_setup
                    .as_ref()
                    .map_or(0, |batch| batch.items.len());
                let surface = if self.core.batch_inspecting() {
                    components::centered_spinner(
                        "batch-inspection",
                        "Checking files",
                        palette,
                        animate,
                    )
                } else {
                    components::inline_message(
                        "Batch configuration and submission are unavailable during the UI migration.",
                        palette,
                    )
                };
                let footer = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(M.action_gap))
                    .child(back.render(palette, window))
                    .child(shell::navigation_actions(queue, settings, palette, window));
                (
                    components::page_title(format!("{count} files selected")),
                    surface,
                    footer,
                )
            }
            NavigationState::Queue { .. } => {
                let ids: HashSet<_> = self
                    .core
                    .queue
                    .scheduler()
                    .waiting_jobs()
                    .map(|job| job.id())
                    .collect();
                self.waiting_focus.retain(|(id, _), _| ids.contains(id));
                for id in ids {
                    for action in WaitingAction::ALL {
                        self.waiting_focus
                            .entry((id, action))
                            .or_insert_with(|| cx.focus_handle().tab_stop(true).tab_index(0));
                    }
                }
                let finished_ids: HashSet<_> = self
                    .core
                    .queue
                    .scheduler()
                    .jobs()
                    .iter()
                    .filter(|job| job.state().is_terminal())
                    .map(|job| job.id())
                    .collect();
                self.finished_focus
                    .retain(|(id, _), _| finished_ids.contains(id));
                for job in self
                    .core
                    .queue
                    .scheduler()
                    .jobs()
                    .iter()
                    .filter(|job| job.state().is_terminal())
                {
                    for action in FinishedAction::ALL
                        .into_iter()
                        .filter(|action| action.allows(job))
                    {
                        self.finished_focus
                            .entry((job.id(), action))
                            .or_insert_with(|| cx.focus_handle().tab_stop(true).tab_index(0));
                    }
                }
                let mut add = Button::new("queue-add-files", "Add files", Tone::Standard);
                if self.file_dialog.is_pending() || self.core.source.is_inspecting() {
                    add.state = ActionState::Busy;
                } else if self.can_add_queue_files() {
                    add.state =
                        ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                            root.add_queue_files(window, cx);
                        })));
                }
                let add_width = add.measured_width(window);
                let add = add.render(palette, window).w(add_width);
                let add = if self.can_add_queue_files() {
                    add.track_focus(&self.open_focus)
                } else {
                    add
                };
                let scheduler = self.core.queue.scheduler();
                let controls = self.queue_controls();
                let mut primary =
                    Button::new("queue-primary", controls.primary_label, Tone::Primary);
                primary.alternate_labels = &[
                    "Start queue",
                    "Stop queue",
                    "Stop after current",
                    "Stopping after current",
                    "Finishing",
                    "Cancelling",
                ];
                if let Some(action) = controls.primary.filter(|_| self.can_navigate()) {
                    primary.state =
                        ActionState::Enabled(Box::new(cx.listener(move |root, _, _, cx| {
                            root.dispatch_queue_action(action, None, cx);
                        })));
                }
                let active_cancel = scheduler.active_job().map(|id| {
                    let cancelling = scheduler.cancellation_pending();
                    let mut cancel = Button::new(
                        ("cancel-active", id.get()),
                        if cancelling {
                            "Cancelling"
                        } else {
                            "Cancel job"
                        },
                        Tone::Standard,
                    );
                    cancel.alternate_labels = &["Cancel job", "Cancelling"];
                    if controls.cancel_job == Some(id) && self.can_navigate() {
                        cancel.state =
                            ActionState::Enabled(Box::new(cx.listener(move |root, _, _, cx| {
                                root.dispatch_queue_action(
                                    QueueAction::CancelCurrent,
                                    Some(id),
                                    cx,
                                );
                            })));
                    }
                    cancel.render(palette, window)
                });
                let mut cancel_queue =
                    Button::new("cancel-queue", "Cancel queue", Tone::Destructive);
                if controls.cancel_queue && self.can_navigate() {
                    cancel_queue.state =
                        ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                            window.focus(&root.queue_cancel_focus, cx);
                            root.request_cancel_queue(window, cx);
                        })));
                }
                let cancel_queue = cancel_queue.render(palette, window);
                let cancel_queue = if controls.cancel_queue && self.can_navigate() {
                    cancel_queue.track_focus(&self.queue_cancel_focus)
                } else {
                    cancel_queue
                };
                let mut clear = Button::new("clear-finished", "Clear finished", Tone::Standard);
                if !finished_ids.is_empty() && self.can_navigate() {
                    clear.state =
                        ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                            root.clear_finished(window, cx);
                        })));
                }
                let clear = clear
                    .render(palette, window)
                    .aria_description("Remove finished job records only. Output files are kept.");
                let start_guidance = if scheduler.waiting_jobs().next().is_some()
                    && !scheduler.is_started()
                    && !scheduler.is_running()
                    && !self.core.queue_start_allowed()
                {
                    Some(if self.core.discovery.ready().is_none() {
                        "Return to the source screen to check or retry media tool discovery."
                    } else {
                        "Wait for the current operation to finish before starting the queue."
                    })
                } else {
                    None
                };
                let back = back.render(palette, window);
                let back = if self.can_navigate() {
                    back.track_focus(&self.queue_back_focus)
                } else {
                    back
                };
                (
                    queue::header(
                        scheduler,
                        back,
                        add,
                        primary.render(palette, window),
                        controls,
                        palette,
                    ),
                    queue::body(
                        scheduler,
                        queue::BodyState {
                            active_cancel,
                            action_error: self
                                .core
                                .queue_action_error
                                .as_deref()
                                .or(start_guidance),
                            expanded: self.expanded_failure,
                            row_error: self
                                .finished_error
                                .as_ref()
                                .map(|(id, error)| (*id, error.as_str())),
                        },
                        palette,
                        window,
                        animate,
                        |id, action| {
                            if !self.can_navigate() {
                                return (ActionState::Disabled, None);
                            }
                            let state = ActionState::Enabled(Box::new(cx.listener(
                                move |root, _, window, cx| {
                                    root.dispatch_waiting_action(id, action, window, cx);
                                },
                            )));
                            (state, self.waiting_focus.get(&(id, action)).cloned())
                        },
                        |id, action| {
                            if !self.can_navigate() {
                                return (ActionState::Disabled, None);
                            }
                            let state = ActionState::Enabled(Box::new(cx.listener(
                                move |root, _, window, cx| {
                                    root.dispatch_finished_action(id, action, window, cx);
                                },
                            )));
                            (state, self.finished_focus.get(&(id, action)).cloned())
                        },
                    ),
                    components::action_bar(palette, [clear, cancel_queue]),
                )
            }
            NavigationState::Settings { .. } => (
                components::page_title("Settings"),
                components::inline_message(
                    "Appearance and settings controls are unavailable during the UI migration.",
                    palette,
                ),
                components::action_bar(palette, [back.render(palette, window)]),
            ),
            NavigationState::QueueEdit { .. } => {
                let edit = self.core.queue_edit();
                let header = div()
                    .flex()
                    .flex_col()
                    .gap(px(M.related))
                    .child(components::page_title("Edit settings"))
                    .children(edit.map(|edit| {
                        file_intake::summary(
                            super::workspace::WorkspacePresentation::new(&edit.media),
                            None,
                            div(),
                            palette,
                        )
                        .child(components::heading(edit.active_operation.label()))
                    }))
                    .child(components::separator(palette));
                if edit.is_some_and(|edit| edit.active_operation == Operation::Remux) {
                    let (body, actions) = self.remux_form(palette, window, cx);
                    (header, body, actions)
                } else {
                    let mut cancel = Button::new(
                        "cancel-queue-edit",
                        super::CANCEL_QUEUE_EDIT_LABEL,
                        Tone::Standard,
                    );
                    if self.can_navigate() {
                        cancel.state =
                            ActionState::Enabled(Box::new(cx.listener(|root, _, window, cx| {
                                root.finish_queue_edit(false, window, cx)
                            })));
                    }
                    (
                        header,
                        components::inline_message(
                            "This operation editor is unavailable during the UI migration.",
                            palette,
                        ),
                        components::action_bar(palette, [cancel.render(palette, window)]),
                    )
                }
            }
            NavigationState::BatchEdit => (
                components::page_title("Editor"),
                components::inline_message(
                    "This editor is unavailable during the UI migration.",
                    palette,
                ),
                div(),
            ),
        };
        let scheduler = self.core.queue.scheduler();
        let work = components::progress(
            "queue-progress",
            queue_progress_state(
                scheduler.run_state(),
                scheduler.is_running(),
                scheduler.cancellation_pending(),
                scheduler
                    .active_progress()
                    .and_then(|progress| progress.fraction),
            ),
            palette,
            animate,
        );
        // Keep the queue indicator's footprint outside the route body, including when idle.
        let footer = div()
            .flex()
            .flex_col()
            .gap(px(M.tight))
            .when(!matches!(route, NavigationState::Queue { .. }), |footer| {
                footer.child(work)
            })
            .child(footer);
        let body = if self.exit == ExitState::Open && !self.queue_cancel_confirming {
            let surface = div()
                .flex()
                .flex_col()
                .w_full()
                .min_w_0()
                .gap(px(M.related))
                .when(
                    landing
                        || (route == NavigationState::BatchSetup && self.core.batch_inspecting()),
                    |surface| surface.items_center().justify_center(),
                )
                .child(surface)
                .children(
                    (landing || has_source || matches!(route, NavigationState::Queue { .. }))
                        .then(|| self.core.source.error())
                        .flatten()
                        .map(|error| {
                            let message = format!(
                                "{} {}",
                                error.message(),
                                error.guidance().unwrap_or_default()
                            );
                            components::inline_message(message, palette)
                        }),
                )
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
                }));
            shell::page(header, surface, footer, self.whole_page_scroll.clone())
        } else {
            div().id("exit-page").size_full().overflow_y_scroll().child(
                div()
                    .flex()
                    .flex_col()
                    .min_h_full()
                    .justify_center()
                    .p(px(M.inset))
                    .child(if self.queue_cancel_confirming {
                        self.cancel_queue_view(palette, window, cx)
                    } else {
                        self.exit_view(palette, window, cx)
                    }),
            )
        };

        let root = TextRole::Body
            .apply(div())
            .id("apollo-root")
            .tab_group()
            .track_focus(&self.focus)
            .size_full()
            .bg(rgba(palette.background))
            .text_color(rgba(palette.foreground))
            .on_key_down(cx.listener(|root, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && root.exit == ExitState::Confirming {
                    root.dismiss_close(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && root.queue_cancel_confirming {
                    root.resolve_cancel_queue(false, window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape"
                    && matches!(
                        root.core.navigation(),
                        NavigationState::Queue { .. }
                            | NavigationState::Settings { .. }
                            | NavigationState::QueueEdit { .. }
                    )
                    && root.can_navigate()
                {
                    root.back(window, cx);
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
            .child(body)
            .border(px(M.rule))
            .border_color(rgba(palette.background))
            .on_file_drop_exit(cx.listener(|root, _, _, _| {
                root.drag_origin = None;
            }))
            .on_drag_move::<ExternalPaths>(cx.listener(
                |root, event: &gpui::DragMoveEvent<ExternalPaths>, _, cx| {
                    if root.drag_origin.is_none() {
                        root.drag_origin = Some(
                            if root.can_open_files() && !event.drag(cx).paths().is_empty() {
                                DropContext::Eligible(InputDialogOrigin::capture(&root.core))
                            } else {
                                DropContext::Blocked
                            },
                        );
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(|root, paths: &ExternalPaths, window, cx| {
                root.accept_drop(paths, window, cx)
            }));
        let valid_drag = match self.drag_origin {
            Some(DropContext::Eligible(origin)) => origin.is_current(&self.core),
            Some(DropContext::Blocked) => false,
            None => true,
        };
        if self.can_open_files() && valid_drag {
            root.drag_over::<ExternalPaths>(move |style, paths, _, _| {
                if paths.paths().is_empty() {
                    style
                } else {
                    style.border_color(rgba(palette.border()))
                }
            })
        } else {
            root
        }
    }
}

fn open_root(cx: &mut App) -> Result<(), String> {
    let fonts = cx.text_system();
    fonts
        .add_fonts(vec![
            Cow::Borrowed(include_bytes!("../../assets/fonts/Geist-Variable.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../../assets/fonts/FiraCode-Regular.ttf").as_slice()),
        ])
        .map_err(|error| format!("Could not register Apollo's fonts: {error:#}"))?;
    let names = fonts.all_font_names();
    for family in [FONT_FAMILY, WORDMARK_FAMILY] {
        if !names.iter().any(|name| name == family) {
            return Err(format!(
                "The bundled {family} font family could not be resolved."
            ));
        }
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
            cx.set_reduce_motion(native::reduce_motion());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::discovery::MediaToolsDiscovery,
        media::{
            intake::MediaLoadStart,
            probe::{MediaProbeError, parse_ffprobe_json},
            tools::{MediaTools, MediaToolsError},
        },
        queue::QueueRunState,
    };
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn landing_discovery_failure_retry_and_base_readiness() {
        let mut core = Application::default();
        let (wake, wakes) = mpsc::channel();
        let failure_wake = wake.clone();
        assert!(core.discovery.start_with(
            move || {
                let _ = failure_wake.send(());
            },
            || Err(MediaToolsError::MissingFfprobe),
            |_| unreachable!()
        ));
        assert_eq!(landing_content(&core), LandingContent::Discovering);
        let timeout = Duration::from_secs(10);
        wakes.recv_timeout(timeout).unwrap();
        assert!(core.poll_background(|| panic!("idle queue must not launch")));
        assert_eq!(landing_content(&core), LandingContent::ToolsUnavailable);
        let (release, gate) = mpsc::channel();
        assert!(core.discovery.start_with(
            move || {
                let _ = wake.send(());
            },
            || Ok(MediaTools::new("ffmpeg", "ffprobe")),
            move |_| {
                gate.recv_timeout(timeout).unwrap();
                Default::default()
            }
        ));
        assert_eq!(landing_content(&core), LandingContent::Discovering);
        wakes.recv_timeout(timeout).unwrap();
        assert!(core.poll_background(|| panic!("idle queue must not launch")));
        assert!(
            core.discovery.is_pending(),
            "encoder discovery is independent of base readiness"
        );
        assert_eq!(landing_content(&core), LandingContent::Ready);
        release.send(()).unwrap();
        wakes.recv_timeout(timeout).unwrap();
        assert!(core.poll_background(|| panic!("idle queue must not launch")));
        assert!(!core.discovery.is_pending());
        assert_eq!(landing_content(&core), LandingContent::Ready);
    }

    #[test]
    fn initial_inspection_reveals_only_accepted_results_and_recovers_after_failure_or_cancel() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.mkv");
        std::fs::write(&path, b"media fixture").unwrap();
        let mut core = Application::default();
        core.discovery = MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
            "ffmpeg", "ffprobe",
        )));
        for outcome in ["failure", "cancel", "success"] {
            let Some(MediaLoadStart::Probe(request)) = core.source.begin_load(&path) else {
                panic!("expected a fresh probe")
            };
            let (sender, receiver) = mpsc::sync_channel(1);
            core.source.attach_inspection(request, receiver);
            assert_eq!(landing_content(&core), LandingContent::Inspecting);
            assert!(core.source.current().is_none());
            match outcome {
                "failure" => {
                    sender
                        .send(Err(MediaProbeError::LaunchFailed(
                            std::io::ErrorKind::NotFound,
                        )))
                        .unwrap();
                    assert!(core.poll_background(|| panic!("idle queue must not launch")));
                    assert!(core.source.error().is_some());
                }
                "cancel" => {
                    core.close_source();
                    let media = parse_ffprobe_json(
                        &path,
                        include_bytes!("../../tests/fixtures/ffprobe/complete_inventory.json"),
                    )
                    .unwrap();
                    assert!(sender.send(Ok(media)).is_err());
                    assert!(!core.poll_background(|| panic!("idle queue must not launch")));
                    assert!(core.source.current().is_none());
                }
                _ => {
                    let media = parse_ffprobe_json(
                        &path,
                        include_bytes!("../../tests/fixtures/ffprobe/complete_inventory.json"),
                    )
                    .unwrap();
                    sender.send(Ok(media)).unwrap();
                    assert!(core.poll_background(|| panic!("idle queue must not launch")));
                    assert!(matches!(
                        core.navigation(),
                        NavigationState::SourceWorkspace { .. }
                    ));
                    assert_eq!(core.source.current().unwrap().path, path);
                }
            }
            assert_eq!(landing_content(&core), LandingContent::Ready);
        }
    }

    #[test]
    fn queue_indicator_follows_active_work_not_pending_count_or_last_progress() {
        let stale = Some(0.5);
        assert_eq!(
            queue_progress_state(QueueRunState::Stopped, false, false, stale),
            ProgressState::Idle
        );
        assert_eq!(
            queue_progress_state(QueueRunState::Running, false, false, stale),
            ProgressState::Waiting("Waiting to process queue")
        );
        for fraction in [None, Some(0.42)] {
            assert_eq!(
                queue_progress_state(QueueRunState::Running, true, false, fraction),
                ProgressState::Active {
                    label: "Processing queue",
                    fraction
                }
            );
            assert_eq!(
                queue_progress_state(QueueRunState::Stopped, true, true, fraction),
                ProgressState::Active {
                    label: "Cancelling queue job",
                    fraction
                }
            );
        }
        assert_eq!(
            queue_progress_state(QueueRunState::Running, true, false, Some(1.)),
            ProgressState::Active {
                label: "Finalizing queue job",
                fraction: None
            }
        );
        assert_eq!(
            queue_progress_state(QueueRunState::Stopped, false, false, None),
            ProgressState::Idle
        );
    }
}

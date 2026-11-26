//! Queue presentation derived directly from the scheduler's retained records.

#[cfg(any(windows, test))]
use crate::{
    app::{Application, Operation, QueueAction},
    media::{
        error::{MediaError, MediaOutputContainer},
        progress::MediaProgress,
    },
    queue::{JobId, MediaQueue, QueueJobFailure, QueueJobState, QueueRunState, QueuedJob},
};
#[cfg(any(windows, test))]
use std::path::Path;

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Processing,
    Waiting,
    Finished,
}

#[cfg(any(windows, test))]
impl Section {
    const ALL: [Self; 3] = [Self::Processing, Self::Waiting, Self::Finished];
    fn of(job: &QueuedJob) -> Self {
        match job.state() {
            QueueJobState::Running { .. } => Self::Processing,
            QueueJobState::Waiting => Self::Waiting,
            QueueJobState::Completed { .. }
            | QueueJobState::Failed(_)
            | QueueJobState::Cancelled => Self::Finished,
        }
    }
    fn jobs(self, jobs: &[QueuedJob]) -> impl Iterator<Item = &QueuedJob> {
        jobs.iter().filter(move |job| Self::of(job) == self)
    }
    fn title(self, count: usize) -> String {
        match self {
            Self::Processing => "Processing".to_owned(),
            Self::Waiting => format!("Waiting ({count})"),
            Self::Finished => format!("Finished ({count})"),
        }
    }
}

#[cfg(any(windows, test))]
fn summary(jobs: &[QueuedJob]) -> String {
    let [processing, waiting, finished] = Section::ALL.map(|section| section.jobs(jobs).count());
    format!("{processing} processing · {waiting} waiting · {finished} finished")
}

#[cfg(any(windows, test))]
fn destination(job: &QueuedJob) -> &Path {
    match job.state() {
        QueueJobState::Running { output } | QueueJobState::Completed { output } => output,
        _ => job.operation().output(),
    }
}

#[cfg(any(windows, test))]
fn status(job: &QueuedJob, cancelling: bool) -> &'static str {
    match job.state() {
        QueueJobState::Running { .. } if cancelling => "Cancelling",
        QueueJobState::Running { .. } => "Processing",
        QueueJobState::Waiting => "Waiting",
        QueueJobState::Completed { .. } => "Completed",
        QueueJobState::Failed(_) => "Failed",
        QueueJobState::Cancelled => "Cancelled",
    }
}

#[cfg(any(windows, test))]
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// Derive availability from the same readiness and ownership checks used at launch.
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Controls {
    pub primary_label: &'static str,
    pub primary: Option<QueueAction>,
    pub status: &'static str,
    pub cancel_job: Option<JobId>,
    pub cancel_queue: bool,
}

#[cfg(any(windows, test))]
impl Controls {
    pub(crate) fn for_app(app: &Application) -> Self {
        Self::new(
            app.queue.scheduler(),
            app.queue_start_allowed(),
            app.discovery.settled_capabilities().is_some(),
        )
    }

    pub(crate) fn new(queue: &MediaQueue, start_allowed: bool, capabilities_ready: bool) -> Self {
        let active = queue.active_job();
        let cancelling = queue.cancellation_pending();
        let waiting = queue.waiting_jobs().next().is_some();
        let ready = queue.next_job_ready(capabilities_ready);
        let (primary_label, primary, status) = if cancelling {
            (
                "Cancelling",
                None,
                match queue.run_state() {
                    QueueRunState::Running => "Running",
                    QueueRunState::StoppingAfterCurrent => "Stopping after current",
                    QueueRunState::Stopped => "Stopped",
                },
            )
        } else {
            match queue.run_state() {
                QueueRunState::Running => (
                    if active.is_some() {
                        "Stop after current"
                    } else {
                        "Stop queue"
                    },
                    Some(QueueAction::StopAfterCurrent),
                    if active.is_some() {
                        "Running"
                    } else {
                        "Checking encoders"
                    },
                ),
                QueueRunState::StoppingAfterCurrent => {
                    ("Stopping after current", None, "Finishing current job")
                }
                QueueRunState::Stopped if active.is_some() => ("Finishing", None, "Stopped"),
                QueueRunState::Stopped => (
                    "Start queue",
                    (start_allowed && queue.can_start(capabilities_ready))
                        .then_some(QueueAction::Start),
                    if waiting && !start_allowed {
                        "Start unavailable"
                    } else if waiting && !ready {
                        "Checking encoders"
                    } else {
                        "Stopped"
                    },
                ),
            }
        };
        Self {
            primary_label,
            primary,
            status,
            cancel_job: active.filter(|_| !cancelling),
            cancel_queue: queue.pending_count() > 0 && !cancelling,
        }
    }

    pub(crate) fn allows(self, action: QueueAction, expected_job: Option<JobId>) -> bool {
        match action {
            QueueAction::Start | QueueAction::StopAfterCurrent => self.primary == Some(action),
            QueueAction::CancelCurrent => expected_job.is_some() && self.cancel_job == expected_job,
            QueueAction::CancelQueue => self.cancel_queue,
            _ => false,
        }
    }
}

/// Only forms that implement the shared QueueEdit mode may be opened from Queue.
#[cfg(any(windows, test))]
pub(crate) const fn editor_migrated(operation: Operation) -> bool {
    match operation {
        Operation::Remux | Operation::AudioMix => true,
        Operation::AudioExtract | Operation::VideoUpscale | Operation::FrameCapture => false,
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum WaitingAction {
    Earlier,
    Later,
    Edit,
    Remove,
}

#[cfg(any(windows, test))]
impl WaitingAction {
    pub(crate) const ALL: [Self; 4] = [Self::Earlier, Self::Later, Self::Edit, Self::Remove];

    #[cfg(windows)]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Earlier => "Move up",
            Self::Later => "Move down",
            Self::Edit => "Edit settings",
            Self::Remove => "Remove",
        }
    }

    pub(crate) fn command(self, id: JobId) -> QueueAction {
        match self {
            Self::Earlier => QueueAction::MoveEarlier(id),
            Self::Later => QueueAction::MoveLater(id),
            Self::Edit => QueueAction::Edit(id),
            Self::Remove => QueueAction::Remove(id),
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WaitingControls {
    pub index: usize,
    count: usize,
    pub edit_blocked: Option<&'static str>,
}

#[cfg(any(windows, test))]
fn edit_unavailable(queue: &MediaQueue, job: &QueuedJob) -> Option<&'static str> {
    if !queue.can_edit() {
        return Some(match queue.run_state() {
            QueueRunState::Running => {
                "Stop the queue and wait for any active job to finish before editing."
            }
            QueueRunState::StoppingAfterCurrent | QueueRunState::Stopped => {
                "Wait for the active job to settle before editing."
            }
        });
    }
    (!editor_migrated(Operation::from(job.operation())))
        .then_some("Editing is unavailable for operations that have not been migrated.")
}

#[cfg(any(windows, test))]
impl WaitingControls {
    fn at(queue: &MediaQueue, job: &QueuedJob, index: usize, count: usize) -> Self {
        let edit_blocked = edit_unavailable(queue, job);
        Self {
            index,
            count,
            edit_blocked,
        }
    }

    pub(crate) fn for_job(queue: &MediaQueue, id: JobId) -> Option<Self> {
        let index = queue.waiting_jobs().position(|job| job.id() == id)?;
        let job = queue.waiting_jobs().nth(index)?;
        Some(Self::at(queue, job, index, queue.waiting_jobs().count()))
    }

    pub(crate) fn allows(self, action: WaitingAction) -> bool {
        match action {
            WaitingAction::Earlier => self.index > 0,
            WaitingAction::Later => self.index + 1 < self.count,
            WaitingAction::Edit => self.edit_blocked.is_none(),
            WaitingAction::Remove => true,
        }
    }
}

/// Keep focus on the moved job, or the nearest surviving waiting row after removal.
#[cfg(any(windows, test))]
pub(crate) fn waiting_focus_after(
    queue: &MediaQueue,
    id: JobId,
    previous_index: usize,
    action: WaitingAction,
) -> Option<(JobId, WaitingAction)> {
    if action == WaitingAction::Remove {
        let job = queue
            .waiting_jobs()
            .nth(previous_index)
            .or_else(|| queue.waiting_jobs().last())?;
        return Some((job.id(), WaitingAction::Remove));
    }
    let controls = WaitingControls::for_job(queue, id)?;
    let action = if controls.allows(action) {
        action
    } else {
        match action {
            WaitingAction::Earlier if controls.allows(WaitingAction::Later) => WaitingAction::Later,
            WaitingAction::Later if controls.allows(WaitingAction::Earlier) => {
                WaitingAction::Earlier
            }
            _ => WaitingAction::Remove,
        }
    };
    Some((id, action))
}

/// FFmpeg may report its last timestamp before publication and cleanup settle.
#[cfg(any(windows, test))]
pub(crate) fn unsettled_fraction(fraction: Option<f32>) -> Option<f32> {
    fraction.filter(|value| value.is_finite() && (0.0..1.0).contains(value))
}

#[cfg(any(windows, test))]
pub(crate) fn job_progress(queue: &MediaQueue, job_id: JobId) -> Option<&MediaProgress> {
    if queue.active_job() == Some(job_id) {
        queue.active_progress()
    } else {
        None
    }
}

#[cfg(any(windows, test))]
pub(crate) fn failure_error(job: &QueuedJob) -> Option<MediaError> {
    let QueueJobState::Failed(failure) = job.state() else {
        return None;
    };
    let error = match failure {
        QueueJobFailure::SourceUnavailable(error) => MediaError::from_intake_error(error),
        QueueJobFailure::SourceChanged => MediaError::source_changed(failure.to_string()),
        QueueJobFailure::Preparation(error) => error.user_error(),
        QueueJobFailure::ProcessStart(error) => MediaError::from_process_start_error(error),
        QueueJobFailure::Execution(error) => {
            let container = match job
                .operation()
                .output()
                .extension()
                .and_then(|ext| ext.to_str())
            {
                Some(ext) if ext.eq_ignore_ascii_case("mp4") => MediaOutputContainer::Mp4,
                Some(ext) if ext.eq_ignore_ascii_case("mkv") => MediaOutputContainer::Mkv,
                _ => MediaOutputContainer::Other,
            };
            MediaError::from_execution_failure(error, container)
        }
    };
    Some(error)
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FinishedAction {
    Open,
    Reveal,
    Retry,
    Details,
    CopyDetails,
}

#[cfg(any(windows, test))]
impl FinishedAction {
    pub(crate) const ALL: [Self; 5] = [
        Self::Open,
        Self::Reveal,
        Self::Retry,
        Self::Details,
        Self::CopyDetails,
    ];

    pub(crate) fn allows(self, job: &QueuedJob) -> bool {
        match job.state() {
            QueueJobState::Completed { .. } => matches!(self, Self::Open | Self::Reveal),
            QueueJobState::Failed(_) => {
                matches!(self, Self::Retry | Self::Details | Self::CopyDetails)
            }
            QueueJobState::Cancelled => self == Self::Retry,
            _ => false,
        }
    }

    #[cfg(windows)]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Open => crate::native::OutputAction::Open.label(),
            Self::Reveal => crate::native::OutputAction::Reveal.label(),
            Self::Retry => "Retry",
            Self::Details => "Details",
            Self::CopyDetails => "Copy details",
        }
    }

    #[cfg(windows)]
    pub(crate) fn command(self, id: JobId) -> Option<QueueAction> {
        match self {
            Self::Open => Some(QueueAction::OpenOutput(id)),
            Self::Reveal => Some(QueueAction::ShowInFolder(id)),
            Self::Retry => Some(QueueAction::Retry(id)),
            Self::Details | Self::CopyDetails => None,
        }
    }
}

#[cfg(any(windows, test))]
pub(crate) fn failure_details(job: &QueuedJob) -> Option<String> {
    let error = failure_error(job)?;
    let mut details = error.message().to_owned();
    if let Some(guidance) = error.guidance() {
        details.push_str("\n\n");
        details.push_str(guidance);
    }
    if let Some(diagnostic) = error.diagnostic() {
        details.push_str("\n\n");
        details.push_str(diagnostic);
    }
    Some(details)
}

#[cfg(windows)]
mod view {
    use super::*;
    use crate::ui::{
        components::{self, ActionState, Button, Tone},
        design::{METRICS as M, Palette, TextRole},
    };
    use gpui::{Div, FocusHandle, Pixels, Role, Stateful, Window, div, prelude::*, px, rgba};

    pub(crate) fn header(
        queue: &MediaQueue,
        back: Stateful<Div>,
        add: Stateful<Div>,
        primary: Stateful<Div>,
        controls: Controls,
        palette: Palette,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(M.related))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(M.related))
                    .child(back.flex_shrink_0())
                    .child(components::page_title("Queue").flex_1().min_w_0())
                    .child(add.flex_shrink_0()),
            )
            .child(
                TextRole::Secondary
                    .apply(div())
                    .text_color(rgba(palette.secondary()))
                    .child(summary(queue.jobs())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(M.related))
                    .child(
                        TextRole::Status
                            .apply(div())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(controls.status),
                    )
                    .child(primary),
            )
            .child(components::separator(palette))
    }
    pub(crate) struct BodyState<'a> {
        pub active_cancel: Option<Stateful<Div>>,
        pub action_error: Option<&'a str>,
        pub expanded: Option<JobId>,
        pub row_error: Option<(JobId, &'a str)>,
    }

    struct RowWidths {
        operation: Pixels,
        status: Pixels,
        stacked: bool,
    }

    pub(crate) fn body(
        queue: &MediaQueue,
        state: BodyState<'_>,
        palette: Palette,
        window: &Window,
        animate: bool,
        mut waiting_action: impl FnMut(JobId, WaitingAction) -> (ActionState, Option<FocusHandle>),
        mut finished_action: impl FnMut(JobId, FinishedAction) -> (ActionState, Option<FocusHandle>),
    ) -> Div {
        let BodyState {
            active_cancel,
            action_error,
            expanded,
            row_error,
        } = state;
        let jobs = queue.jobs();
        if jobs.is_empty() && action_error.is_none() {
            return div()
                .flex()
                .flex_col()
                .justify_center()
                .flex_1()
                .w_full()
                .child(components::empty_state(
                    "No jobs yet",
                    "Go back to select a source or add files.",
                    palette,
                ));
        }
        let text = window.text_system();
        let operation_width = Operation::ALL
            .into_iter()
            .map(|operation| components::text_width(text, TextRole::Secondary, operation.label()))
            .fold(px(0.), Pixels::max);
        let status_width = [
            "Processing",
            "Cancelling",
            "Waiting",
            "✓ Completed",
            "× Failed",
            "– Cancelled",
        ]
        .into_iter()
        .map(|label| components::text_width(text, TextRole::Status, label))
        .fold(px(0.), Pixels::max);
        let status_width = status_width.max(
            components::text_width(text, TextRole::Status, "× Failed")
                + px(M.compact)
                + Button::new("measure-details", "Details", Tone::Standard).measured_width(window),
        );
        let widths = RowWidths {
            operation: operation_width,
            status: status_width,
            stacked: window.viewport_size().width
                < operation_width + status_width + px(2. * M.inset + 3. * M.related + 140.),
        };
        let mut active_cancel = active_cancel;
        div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .gap(px(M.section))
            .children(
                action_error.map(|error| components::inline_message(error.to_owned(), palette)),
            )
            .children(Section::ALL.into_iter().filter_map(|section| {
                let count = section.jobs(jobs).count();
                if count == 0 {
                    return None;
                }
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(M.compact))
                        .child(components::heading(section.title(count)))
                        .children(
                            (section == Section::Waiting)
                                .then(|| {
                                    section
                                        .jobs(jobs)
                                        .find_map(|job| edit_unavailable(queue, job))
                                })
                                .flatten()
                                .map(|reason| {
                                    TextRole::Secondary
                                        .apply(div())
                                        .text_color(rgba(palette.secondary()))
                                        .child(reason)
                                }),
                        )
                        .children(section.jobs(jobs).enumerate().map(|(index, job)| {
                            let active = queue.active_job() == Some(job.id());
                            let cancelling = active && queue.cancellation_pending();
                            let details = FinishedAction::Details.allows(job).then(|| {
                                finished_button(
                                    job,
                                    FinishedAction::Details,
                                    expanded,
                                    palette,
                                    window,
                                    &mut finished_action,
                                )
                            });
                            let details = row(job, cancelling, palette, &widths, details);
                            let mut content =
                                div().flex().flex_col().gap(px(M.compact)).child(details);
                            if active {
                                let progress = job_progress(queue, job.id());
                                let fraction = progress.and_then(|p| p.fraction);
                                let label = if cancelling {
                                    "Cancelling"
                                } else if fraction.is_some_and(|f| f.is_finite() && f >= 1.) {
                                    "Finalizing output"
                                } else {
                                    "Processing"
                                };
                                let timing = progress.and_then(|p| {
                                    p.processed.map(|processed| {
                                        let processed =
                                            super::super::media_lifecycle::format_duration(
                                                processed,
                                            );
                                        match p.total {
                                            Some(total) => format!(
                                                "Processed {processed} / {}",
                                                super::super::media_lifecycle::format_duration(
                                                    total
                                                )
                                            ),
                                            None => format!("Processed {processed}"),
                                        }
                                    })
                                });
                                content = content.child(
                                    div()
                                        .id(("queue-active", job.id().get()))
                                        .flex()
                                        .flex_col()
                                        .gap(px(M.tight))
                                        .child(components::progress(
                                            "job-progress",
                                            components::ProgressState::Active {
                                                label,
                                                fraction: unsettled_fraction(fraction),
                                            },
                                            palette,
                                            animate,
                                        ))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(M.related))
                                                .child(
                                                    TextRole::Secondary
                                                        .apply(div())
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child(timing.unwrap_or_default()),
                                                )
                                                .children(active_cancel.take()),
                                        ),
                                );
                            }
                            if section == Section::Waiting {
                                content = content.child(waiting_actions(
                                    job,
                                    WaitingControls::at(queue, job, index, count),
                                    palette,
                                    window,
                                    &mut waiting_action,
                                ));
                            }
                            if section == Section::Finished {
                                let buttons = [
                                    FinishedAction::Open,
                                    FinishedAction::Reveal,
                                    FinishedAction::Retry,
                                ]
                                .into_iter()
                                .filter(|action| action.allows(job))
                                .map(|action| {
                                    finished_button(
                                        job,
                                        action,
                                        expanded,
                                        palette,
                                        window,
                                        &mut finished_action,
                                    )
                                });
                                content =
                                    content.child(components::selector(buttons).justify_end());
                                if expanded == Some(job.id())
                                    && let Some(error) = failure_error(job)
                                {
                                    content = content.child(failure_panel(
                                        job,
                                        &error,
                                        palette,
                                        finished_button(
                                            job,
                                            FinishedAction::CopyDetails,
                                            expanded,
                                            palette,
                                            window,
                                            &mut finished_action,
                                        ),
                                    ));
                                }
                                if let Some((id, error)) = row_error
                                    && id == job.id()
                                {
                                    content = content.child(components::inline_message(
                                        error.to_owned(),
                                        palette,
                                    ));
                                }
                            }
                            content.child(components::separator(palette))
                        })),
                )
            }))
    }
    fn waiting_actions(
        job: &QueuedJob,
        controls: WaitingControls,
        palette: Palette,
        window: &Window,
        action_state: &mut impl FnMut(JobId, WaitingAction) -> (ActionState, Option<FocusHandle>),
    ) -> Div {
        let id = job.id();
        let name = file_name(job.operation().input());
        let output = destination(job).to_string_lossy();
        let buttons = WaitingAction::ALL.map(|action| {
            let mut button =
                Button::new((action.label(), id.get()), action.label(), Tone::Standard);
            let focus = if controls.allows(action) {
                let (state, focus) = action_state(id, action);
                button.state = state;
                focus
            } else {
                None
            };
            let mut element = button.render(palette, window).aria_label(format!(
                "{} for {} ({})",
                action.label(),
                name,
                job.operation().display_name()
            ));
            let description = if action == WaitingAction::Edit {
                controls
                    .edit_blocked
                    .map(|reason| format!("{reason} Destination: {output}"))
            } else {
                None
            };
            element = element
                .aria_description(description.unwrap_or_else(|| format!("Destination: {output}")));
            if let Some(focus) = focus {
                element = element.track_focus(&focus);
            }
            element
        });
        components::selector(buttons).justify_end()
    }

    fn finished_button(
        job: &QueuedJob,
        action: FinishedAction,
        expanded: Option<JobId>,
        palette: Palette,
        window: &Window,
        action_state: &mut impl FnMut(JobId, FinishedAction) -> (ActionState, Option<FocusHandle>),
    ) -> Stateful<Div> {
        let (state, focus) = action_state(job.id(), action);
        let mut button = Button::new(
            (action.label(), job.id().get()),
            action.label(),
            Tone::Standard,
        );
        button.state = match state {
            ActionState::Enabled(callback)
                if action == FinishedAction::Details && expanded == Some(job.id()) =>
            {
                ActionState::Selected(callback)
            }
            state => state,
        };
        let mut element = button.render(palette, window).aria_label(format!(
            "{} for {} ({})",
            action.label(),
            file_name(job.operation().input()),
            job.operation().display_name()
        ));
        if action == FinishedAction::Details {
            element = element.aria_description(if expanded == Some(job.id()) {
                "Collapse failure details"
            } else {
                "Expand failure details"
            });
        }
        if let Some(focus) = focus {
            element = element.track_focus(&focus);
        }
        element
    }

    fn failure_panel(
        job: &QueuedJob,
        error: &MediaError,
        palette: Palette,
        copy: Stateful<Div>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(M.tight))
            .min_w_0()
            .child(components::inline_message(error.message(), palette))
            .children(
                error
                    .guidance()
                    .map(|guidance| components::inline_message(guidance, palette)),
            )
            .children(error.diagnostic().map(|diagnostic| {
                div()
                    .id(("queue-diagnostic", job.id().get()))
                    .w_full()
                    .min_w_0()
                    .max_h(px(crate::ui::layout::DIAGNOSTIC_MAX_HEIGHT))
                    .overflow_scroll()
                    .border(px(M.rule))
                    .border_color(rgba(palette.border()))
                    .p(px(M.compact))
                    .child(
                        TextRole::Secondary
                            .apply(div())
                            .whitespace_normal()
                            .child(diagnostic.to_owned()),
                    )
            }))
            .child(components::selector([copy]).justify_end())
    }

    fn row(
        job: &QueuedJob,
        cancelling: bool,
        palette: Palette,
        widths: &RowWidths,
        details: Option<Stateful<Div>>,
    ) -> Stateful<Div> {
        let source = job.operation().input();
        let output = destination(job);
        let source_name = file_name(source);
        let operation = job.operation().display_name();
        let state = status(job, cancelling);
        let displayed_status = match job.state() {
            QueueJobState::Completed { .. } => "✓ Completed",
            QueueJobState::Failed(_) => "× Failed",
            QueueJobState::Cancelled => "– Cancelled",
            _ => state,
        };
        let output_path = output.to_string_lossy().into_owned();
        let destination_label = if matches!(
            job.state(),
            QueueJobState::Failed(_) | QueueJobState::Cancelled
        ) {
            format!("Requested: {output_path}")
        } else {
            format!("→ {output_path}")
        };
        div()
            .id(("queue-job", job.id().get()))
            .role(Role::Group)
            .aria_label(format!("{source_name}, {operation}, {state}"))
            .aria_description(destination_label.clone())
            .flex()
            .items_center()
            .gap(px(M.related))
            .min_w_0()
            .when(widths.stacked, |row| row.flex_col().items_start().w_full())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(px(M.tight))
                    .when(widths.stacked, |identity| identity.w_full())
                    .when(!widths.stacked, |identity| identity.flex_1())
                    .child(components::with_tooltip(
                        TextRole::Body
                            .apply(div())
                            .id("source-name")
                            .truncate()
                            .child(source_name.clone()),
                        source_name,
                        palette,
                    ))
                    .child(components::with_tooltip(
                        TextRole::Secondary
                            .apply(div())
                            .id("destination")
                            .text_color(rgba(palette.secondary()))
                            .text_ellipsis_middle()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(destination_label),
                        output_path,
                        palette,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(M.related))
                    .flex_shrink_0()
                    .when(widths.stacked, |metadata| {
                        metadata.w_full().justify_between()
                    })
                    .child(
                        TextRole::Secondary
                            .apply(div())
                            .w(widths.operation)
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .text_color(rgba(palette.secondary()))
                            .child(operation),
                    )
                    .child(components::status(displayed_status, palette, details).w(widths.status)),
            )
    }
}
#[cfg(windows)]
pub(crate) use view::{BodyState, body, header};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audio_extract::model::{AudioExtractFormat, AudioExtractRequest},
        media::{container::MediaContainer, intake::SourceIdentity, model::MediaFile},
        queue::QueuedOperation,
        remux::model::RemuxRequest,
    };
    fn job(id: u64, source: &Path, output: &Path, state: QueueJobState) -> QueuedJob {
        let mut job = QueuedJob::new(
            JobId::new(id),
            SourceIdentity::read(source).unwrap(),
            MediaFile {
                path: source.to_owned(),
                duration: None,
                streams: vec![],
                video_streams: vec![],
                audio_streams: vec![],
            },
            QueuedOperation::Remux(RemuxRequest {
                input: source.to_owned(),
                output: output.to_owned(),
                container: MediaContainer::Matroska,
            }),
        );
        job.set_state(state);
        job
    }
    #[test]
    fn empty_application_disables_scheduler_actions() {
        let controls = Controls::for_app(&Application::default());
        assert_eq!(controls.primary_label, "Start queue");
        assert!(!controls.allows(QueueAction::Start, None));
        assert!(!controls.allows(QueueAction::CancelQueue, None));
        assert!(!controls.allows(QueueAction::CancelCurrent, Some(JobId::new(1))));
    }

    #[test]
    fn progress_never_claims_completion_before_settlement() {
        for fraction in [None, Some(f32::NAN), Some(-0.1), Some(1.), Some(1.1)] {
            assert_eq!(unsettled_fraction(fraction), None);
        }
        assert_eq!(unsettled_fraction(Some(0.)), Some(0.));
        assert_eq!(unsettled_fraction(Some(0.999)), Some(0.999));
    }

    #[test]
    fn failure_details_use_the_bounded_diagnostic_and_actionable_reason() {
        use crate::media::{
            execution::MediaExecutionFailure,
            process::{DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES, MediaProcessFailure},
        };
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"media").unwrap();
        let output = dir.path().join("out.mkv");
        let diagnostic = format!(
            "{}\nfinal error marker",
            "x".repeat(DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES * 2)
        );
        let failed = job(
            1,
            &source,
            &output,
            QueueJobState::Failed(QueueJobFailure::Execution(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic,
            })),
        );
        let error = failure_error(&failed).unwrap();
        let retained = error.diagnostic().unwrap();
        assert!(retained.len() <= DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES);
        let details = failure_details(&failed).unwrap();
        assert!(details.starts_with(error.message()));
        assert!(details.contains(error.guidance().unwrap()));
        assert!(details.ends_with("final error marker"));
    }

    #[test]
    fn groups_preserve_job_identity_and_scheduler_order_across_states() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a long source filename with spaces.mkv");
        std::fs::write(&source, b"media").unwrap();
        let requested = dir.path().join("output.mkv");
        let actual = dir.path().join("output (1).mkv");
        let mut jobs = [
            job(
                1,
                &source,
                &requested,
                QueueJobState::Completed {
                    output: actual.clone(),
                },
            ),
            job(4, &source, &requested, QueueJobState::Waiting),
            job(
                2,
                &source,
                &requested,
                QueueJobState::Running {
                    output: actual.clone(),
                },
            ),
            job(3, &source, &requested, QueueJobState::Waiting),
            job(
                5,
                &source,
                &requested,
                QueueJobState::Failed(QueueJobFailure::SourceChanged),
            ),
            job(6, &source, &requested, QueueJobState::Cancelled),
        ];
        let audio_output = dir.path().join("output.m4a");
        jobs[3].replace_operation(QueuedOperation::AudioExtract(AudioExtractRequest {
            input: source.clone(),
            output: audio_output.clone(),
            audio_stream_index: 1,
            format: AudioExtractFormat::Aac,
        }));
        let ids = |section: Section| {
            section
                .jobs(&jobs)
                .map(|job| job.id().get())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Section::Processing), [2]);
        assert_eq!(ids(Section::Waiting), [4, 3]);
        assert_eq!(ids(Section::Finished), [1, 5, 6]);
        assert_eq!(summary(&jobs), "1 processing · 2 waiting · 3 finished");
        assert!(
            failure_details(&jobs[4])
                .unwrap()
                .contains("source has changed")
        );
        let allowed = |job: &QueuedJob| {
            FinishedAction::ALL
                .into_iter()
                .filter(|action| action.allows(job))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            allowed(&jobs[0]),
            [FinishedAction::Open, FinishedAction::Reveal]
        );
        assert_eq!(
            allowed(&jobs[4]),
            [
                FinishedAction::Retry,
                FinishedAction::Details,
                FinishedAction::CopyDetails
            ]
        );
        assert_eq!(allowed(&jobs[5]), [FinishedAction::Retry]);
        assert!(allowed(&jobs[1]).is_empty());
        assert!(allowed(&jobs[2]).is_empty());
        assert!(failure_details(&jobs[0]).is_none());
        assert_eq!(summary(&[]), "0 processing · 0 waiting · 0 finished");
        assert_eq!(
            summary(&jobs[0..1]),
            "0 processing · 0 waiting · 1 finished"
        );
        assert_eq!(
            summary(&jobs[1..2]),
            "0 processing · 1 waiting · 0 finished"
        );
        assert_eq!(
            summary(&jobs[2..3]),
            "1 processing · 0 waiting · 0 finished"
        );
        assert_eq!(Section::Waiting.title(2), "Waiting (2)");
        assert_eq!(destination(&jobs[0]), actual);
        assert_eq!(destination(&jobs[2]), actual);
        assert_eq!(destination(&jobs[1]), requested);
        assert_eq!(destination(&jobs[3]), audio_output);
        assert_eq!(jobs[3].operation().display_name(), "Audio Extract");
        assert_eq!(destination(&jobs[4]), requested);
        assert_eq!(status(&jobs[2], true), "Cancelling");
        assert_eq!(status(&jobs[2], false), "Processing");
        assert_eq!(status(&jobs[0], true), "Completed");
        assert_eq!(status(&jobs[4], false), "Failed");
        assert_eq!(status(&jobs[5], false), "Cancelled");
        assert_eq!(file_name(&source), "a long source filename with spaces.mkv");
    }
}

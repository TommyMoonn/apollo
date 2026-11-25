//! Queue presentation derived directly from the scheduler's retained records.

#[cfg(any(windows, test))]
use crate::{
    app::{Application, QueueAction},
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
fn failure_copy(job: &QueuedJob) -> Option<String> {
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
    Some(
        format!(
            "{} {}",
            error.message(),
            error.guidance().unwrap_or_default()
        )
        .trim()
        .to_owned(),
    )
}

#[cfg(windows)]
mod view {
    use super::*;
    use crate::{
        app::Operation,
        ui::{
            components,
            design::{METRICS as M, Palette, TextRole},
        },
    };
    use gpui::{Div, Pixels, Role, Stateful, Window, div, prelude::*, px, rgba};

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
    pub(crate) fn body(
        queue: &MediaQueue,
        active_cancel: Option<Stateful<Div>>,
        action_error: Option<&str>,
        palette: Palette,
        window: &Window,
        animate: bool,
    ) -> Div {
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
            "Completed",
            "Failed",
            "Cancelled",
        ]
        .into_iter()
        .map(|label| components::text_width(text, TextRole::Status, label))
        .fold(px(0.), Pixels::max);
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
                        .children(section.jobs(jobs).map(|job| {
                            let active = queue.active_job() == Some(job.id());
                            let cancelling = active && queue.cancellation_pending();
                            let details =
                                row(job, cancelling, palette, operation_width, status_width);
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
                            content.child(components::separator(palette))
                        })),
                )
            }))
    }
    fn row(
        job: &QueuedJob,
        cancelling: bool,
        palette: Palette,
        operation_width: Pixels,
        status_width: Pixels,
    ) -> Div {
        let source = job.operation().input();
        let output = destination(job);
        let source_name = file_name(source);
        let operation = job.operation().display_name();
        let state = status(job, cancelling);
        let source_path = source.to_string_lossy().into_owned();
        let output_path = output.to_string_lossy().into_owned();
        div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .gap(px(M.tight))
            .child(
                div()
                    .id(("queue-job", job.id().get()))
                    .role(Role::Group)
                    .aria_label(format!("{source_name}, {operation}, {state}"))
                    .aria_description(format!("Source: {source_path}. Destination: {output_path}"))
                    .flex()
                    .items_center()
                    .gap(px(M.related))
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(M.tight))
                            .child(
                                TextRole::Body
                                    .apply(div())
                                    .id("source-name")
                                    .truncate()
                                    .child(source_name)
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            components::PathTooltip(source_path.clone(), palette)
                                        })
                                        .into()
                                    }),
                            )
                            .child(
                                TextRole::Secondary
                                    .apply(div())
                                    .id("destination")
                                    .text_color(rgba(palette.secondary()))
                                    .text_ellipsis_middle()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(format!("→ {output_path}"))
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            components::PathTooltip(output_path.clone(), palette)
                                        })
                                        .into()
                                    }),
                            ),
                    )
                    .child(
                        TextRole::Secondary
                            .apply(div())
                            .w(operation_width)
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .text_color(rgba(palette.secondary()))
                            .child(operation),
                    )
                    .child(components::status(state, palette, None).w(status_width)),
            )
            .children(failure_copy(job).map(|message| components::inline_message(message, palette)))
    }
}
#[cfg(windows)]
pub(crate) use view::{body, header};

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
            failure_copy(&jobs[4])
                .unwrap()
                .contains("source has changed")
        );
        assert!(failure_copy(&jobs[0]).is_none());
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

//! Queue presentation derived directly from the scheduler's retained records.

#[cfg(any(windows, test))]
use crate::queue::{QueueJobState, QueuedJob};
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

#[cfg(windows)]
mod view {
    use super::*;
    use crate::{
        app::Operation,
        queue::MediaQueue,
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
            .child(components::separator(palette))
    }
    pub(crate) fn body(queue: &MediaQueue, palette: Palette, window: &Window) -> Div {
        let jobs = queue.jobs();
        if jobs.is_empty() {
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
        div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .gap(px(M.section))
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
                            row(
                                job,
                                queue.active_job() == Some(job.id())
                                    && queue.cancellation_pending(),
                                palette,
                                operation_width,
                                status_width,
                            )
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
            .child(components::separator(palette))
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
        queue::{JobId, QueueJobFailure, QueuedOperation},
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

use std::{cell::Cell, path::Path};

use egui::{Label, RichText, Sides, TextStyle, Ui};

use crate::{
    media::progress::MediaProgress,
    queue::{JobId, QueueJobState, QueuedJob, QueuedOperation},
};

use super::{QueueAction, fonts, layout, media_lifecycle, theme, widgets};

const TITLE: &str = "Queue";
const EMPTY_COPY: &str = "No jobs queued.";
const START_LABEL: &str = "Start queue";
const CANCEL_LABEL: &str = "Cancel current";
const STOP_LABEL: &str = "Stop queue";
const REMOVE_LABEL: &str = "Remove";
const CLEAR_LABEL: &str = "Clear finished";
const BACK_LABEL: &str = "Back";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueCapabilities {
    has_waiting: bool,
    has_terminal: bool,
}

impl QueueCapabilities {
    fn from_jobs(jobs: &[QueuedJob]) -> Self {
        Self {
            has_waiting: jobs
                .iter()
                .any(|job| matches!(job.state(), QueueJobState::Waiting)),
            has_terminal: jobs.iter().any(|job| job.state().is_terminal()),
        }
    }
}

pub fn show(
    ui: &mut Ui,
    jobs: &[QueuedJob],
    active_job: Option<JobId>,
    active_progress: Option<&MediaProgress>,
    queue_started: bool,
    queue_start_allowed: bool,
) -> QueueAction {
    let action = Cell::new(QueueAction::None);
    let capabilities = QueueCapabilities::from_jobs(jobs);

    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            show_header(ui, jobs.len(), capabilities.has_terminal, &action);
            ui.add_space(theme::SPACING.related);

            if jobs.is_empty() {
                media_lifecycle::secondary_copy(ui, EMPTY_COPY);
            } else {
                for job in jobs {
                    widgets::separator(ui);
                    ui.add_space(theme::SPACING.compact);
                    show_job_row(ui, job, active_job, active_progress, &action);
                    ui.add_space(theme::SPACING.compact);
                }
                widgets::separator(ui);
            }
        },
        |ui| {
            if active_job.is_some() {
                if widgets::destructive_button(ui, CANCEL_LABEL).clicked() {
                    action.set(QueueAction::CancelCurrent);
                }
                if widgets::destructive_button(ui, STOP_LABEL).clicked() {
                    action.set(QueueAction::Stop);
                }
            } else if capabilities.has_waiting
                && !queue_started
                && ui
                    .add_enabled_ui(queue_start_allowed, |ui| {
                        widgets::primary_button(ui, START_LABEL)
                    })
                    .inner
                    .clicked()
            {
                action.set(QueueAction::Start);
            }

            if widgets::secondary_button(ui, BACK_LABEL).clicked() {
                action.set(QueueAction::Back);
            }
        },
    );

    action.get()
}

fn show_header(ui: &mut Ui, job_count: usize, can_clear: bool, action: &Cell<QueueAction>) {
    Sides::new().height(theme::COMPACT_CONTROL_HEIGHT).show(
        ui,
        |ui| {
            ui.add(
                Label::new(RichText::new(TITLE).text_style(TextStyle::Heading)).selectable(false),
            );
            ui.add_space(theme::SPACING.compact);
            media_lifecycle::technical_faint(ui, &job_count_copy(job_count));
        },
        |ui| {
            if can_clear && widgets::plain_button(ui, CLEAR_LABEL).clicked() {
                action.set(QueueAction::ClearFinished);
            }
        },
    );
}

fn show_job_row(
    ui: &mut Ui,
    job: &QueuedJob,
    active_job: Option<JobId>,
    active_progress: Option<&MediaProgress>,
    action: &Cell<QueueAction>,
) {
    match layout::queue_row_layout(ui.available_width()) {
        layout::QueueRowLayout::Inline => show_inline_job_row(ui, job, action),
        layout::QueueRowLayout::Stacked => show_stacked_job_row(ui, job, action),
    }

    if let Some(progress) =
        bound_active_progress(job.id(), job.state(), active_job, active_progress)
    {
        ui.add_space(theme::SPACING.compact);
        media_lifecycle::show_progress(ui, &media_lifecycle::progress_presentation(progress));
    }
}

fn show_inline_job_row(ui: &mut Ui, job: &QueuedJob, action: &Cell<QueueAction>) {
    ui.columns(5, |columns| {
        truncated_body(&mut columns[0], file_name(&job.media().path));
        secondary_text(&mut columns[1], operation_label(job.operation()));
        technical_text(&mut columns[2], file_name(job.operation().output()));
        secondary_text(&mut columns[3], state_label(job.state()));
        show_remove(&mut columns[4], job, action);
    });
}

fn show_stacked_job_row(ui: &mut Ui, job: &QueuedJob, action: &Cell<QueueAction>) {
    Sides::new().truncate().show(
        ui,
        |ui| truncated_body(ui, file_name(&job.media().path)),
        |ui| show_remove(ui, job, action),
    );
    ui.add_space(theme::SPACING.tight);
    Sides::new().truncate().show(
        ui,
        |ui| secondary_text(ui, operation_label(job.operation())),
        |ui| secondary_text(ui, state_label(job.state())),
    );
    ui.add_space(theme::SPACING.tight);
    technical_text(ui, file_name(job.operation().output()));
}

fn show_remove(ui: &mut Ui, job: &QueuedJob, action: &Cell<QueueAction>) {
    if matches!(job.state(), QueueJobState::Waiting)
        && widgets::plain_button(ui, REMOVE_LABEL).clicked()
    {
        action.set(QueueAction::Remove(job.id()));
    }
}

fn truncated_body(ui: &mut Ui, text: String) {
    ui.add(
        Label::new(RichText::new(text).text_style(TextStyle::Body))
            .truncate()
            .selectable(false),
    );
}

fn secondary_text(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(TextStyle::Small)
                .color(theme::SECONDARY),
        )
        .truncate()
        .selectable(false),
    );
}

fn technical_text(ui: &mut Ui, text: String) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(fonts::technical_text_style())
                .color(theme::SECONDARY_FAINT),
        )
        .truncate()
        .selectable(false),
    );
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn operation_label(operation: &QueuedOperation) -> &'static str {
    match operation {
        QueuedOperation::AudioMix(_) => "Audio Mix",
        QueuedOperation::Upscale(_) => "Upscale",
        QueuedOperation::Remux(_) => "Remux",
        QueuedOperation::AudioExtract(_) => "Audio Extract",
        QueuedOperation::FrameCapture(_) => "Frame Capture",
    }
}

fn state_label(state: &QueueJobState) -> &'static str {
    match state {
        QueueJobState::Waiting => "Waiting",
        QueueJobState::Running => "Running",
        QueueJobState::Completed { .. } => "Completed",
        QueueJobState::Failed(_) => "Failed",
        QueueJobState::Cancelled => "Cancelled",
    }
}

fn job_count_copy(count: usize) -> String {
    if count == 1 {
        "1 job".to_owned()
    } else {
        format!("{count} jobs")
    }
}

fn bound_active_progress<'a>(
    job_id: JobId,
    state: &QueueJobState,
    active_job: Option<JobId>,
    active_progress: Option<&'a MediaProgress>,
) -> Option<Option<&'a MediaProgress>> {
    (active_job == Some(job_id) && matches!(state, QueueJobState::Running))
        .then_some(active_progress)
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use crate::{
        media::progress::MediaProgress,
        queue::{JobId, QueueJobFailure, QueueJobState},
    };

    use super::{bound_active_progress, file_name, job_count_copy, state_label};

    #[test]
    fn active_progress_binds_only_to_the_running_job_id() {
        let progress = MediaProgress {
            processed: Some(Duration::from_secs(2)),
            total: Some(Duration::from_secs(4)),
            fraction: Some(0.5),
            frame: None,
        };
        let first = JobId::new(1);
        let second = JobId::new(2);

        assert_eq!(
            bound_active_progress(first, &QueueJobState::Running, Some(first), Some(&progress)),
            Some(Some(&progress))
        );
        assert_eq!(
            bound_active_progress(first, &QueueJobState::Running, Some(first), None),
            Some(None)
        );
        assert_eq!(
            bound_active_progress(
                second,
                &QueueJobState::Running,
                Some(first),
                Some(&progress)
            ),
            None
        );
        assert_eq!(
            bound_active_progress(
                first,
                &QueueJobState::Cancelled,
                Some(first),
                Some(&progress)
            ),
            None
        );
    }

    #[test]
    fn queue_copy_covers_every_terminal_and_nonterminal_state() {
        assert_eq!(state_label(&QueueJobState::Waiting), "Waiting");
        assert_eq!(state_label(&QueueJobState::Running), "Running");
        assert_eq!(
            state_label(&QueueJobState::Completed {
                output: PathBuf::from("output.mkv")
            }),
            "Completed"
        );
        assert_eq!(
            state_label(&QueueJobState::Failed(QueueJobFailure::SourceChanged)),
            "Failed"
        );
        assert_eq!(state_label(&QueueJobState::Cancelled), "Cancelled");
        assert_eq!(job_count_copy(0), "0 jobs");
        assert_eq!(job_count_copy(1), "1 job");
        assert_eq!(job_count_copy(2), "2 jobs");
    }

    #[test]
    fn row_filenames_do_not_expand_to_full_paths() {
        assert_eq!(
            file_name(&PathBuf::from("folder/source file.mkv")),
            "source file.mkv"
        );
    }
}

use std::{cell::Cell, path::Path};

use egui::{Align, CollapsingHeader, Label, Layout, RichText, Sides, TextStyle, Ui, vec2};

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
const OPEN_FOLDER_LABEL: &str = "Open folder";
const DETAILS_LABEL: &str = "Details";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueCapabilities {
    has_waiting: bool,
    has_terminal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct QueueSummary {
    running: usize,
    waiting: usize,
    finished: usize,
}

impl QueueSummary {
    fn from_jobs(jobs: &[QueuedJob]) -> Self {
        let mut summary = Self::default();
        for job in jobs {
            match job.state() {
                QueueJobState::Waiting => summary.waiting += 1,
                QueueJobState::Running => summary.running += 1,
                state if state.is_terminal() => summary.finished += 1,
                _ => {}
            }
        }
        summary
    }

    fn copy(self) -> String {
        let mut parts = Vec::new();
        if self.running > 0 {
            parts.push(format!("{} running", self.running));
        }
        if self.waiting > 0 {
            parts.push(format!("{} waiting", self.waiting));
        }
        if self.finished > 0 {
            parts.push(format!("{} finished", self.finished));
        }
        if parts.is_empty() {
            "0 jobs".to_owned()
        } else {
            parts.join(" · ")
        }
    }
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
    let summary = QueueSummary::from_jobs(jobs);

    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            show_header(ui, &summary.copy(), capabilities.has_terminal, &action);
            ui.add_space(theme::SPACING.related);

            if jobs.is_empty() {
                media_lifecycle::secondary_copy(ui, EMPTY_COPY);
            } else {
                let action_width = row_action_width(ui, jobs);
                for job in jobs {
                    widgets::separator(ui);
                    ui.add_space(theme::SPACING.compact);
                    show_job_row(ui, job, active_job, active_progress, action_width, &action);
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

fn show_header(ui: &mut Ui, summary: &str, can_clear: bool, action: &Cell<QueueAction>) {
    Sides::new().height(theme::COMPACT_CONTROL_HEIGHT).show(
        ui,
        |ui| {
            ui.add(
                Label::new(RichText::new(TITLE).text_style(TextStyle::Heading)).selectable(false),
            );
            ui.add_space(theme::SPACING.compact);
            media_lifecycle::technical_faint(ui, summary);
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
    action_width: f32,
    action: &Cell<QueueAction>,
) {
    let plan = layout::queue_row_plan(ui.available_width(), action_width);
    match plan.layout {
        layout::QueueRowLayout::Inline => show_inline_job_row(ui, job, plan, action),
        layout::QueueRowLayout::Stacked => show_stacked_job_row(ui, job, action),
    }

    if let Some(progress) =
        bound_active_progress(job.id(), job.state(), active_job, active_progress)
    {
        ui.add_space(theme::SPACING.compact);
        media_lifecycle::show_progress(ui, &media_lifecycle::progress_presentation(progress));
    }

    if let Some(details) = failure_details(job.state()) {
        ui.add_space(theme::SPACING.tight);
        CollapsingHeader::new(DETAILS_LABEL)
            .id_salt(("queue-failure", job.id().get()))
            .default_open(false)
            .show(ui, |ui| media_lifecycle::diagnostic_viewport(ui, &details));
    }
}

fn show_inline_job_row(
    ui: &mut Ui,
    job: &QueuedJob,
    plan: layout::QueueRowPlan,
    action: &Cell<QueueAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACING.related;
        ui.allocate_ui_with_layout(
            vec2(plan.identity_width, plan.row_height),
            Layout::top_down(Align::Min).with_main_align(Align::Center),
            |ui| show_identity(ui, job),
        );
        ui.allocate_ui_with_layout(
            vec2(plan.operation_width, plan.row_height),
            Layout::left_to_right(Align::Center),
            |ui| secondary_text(ui, operation_label(job.operation())),
        );
        ui.allocate_ui_with_layout(
            vec2(plan.state_width, plan.row_height),
            Layout::left_to_right(Align::Center),
            |ui| state_text(ui, job.state()),
        );
        if plan.action_width > 0.0 {
            ui.allocate_ui_with_layout(
                vec2(plan.action_width, plan.row_height),
                Layout::right_to_left(Align::Center),
                |ui| show_row_action(ui, job, action),
            );
        }
    });
}

fn show_stacked_job_row(ui: &mut Ui, job: &QueuedJob, action: &Cell<QueueAction>) {
    show_identity(ui, job);
    ui.add_space(theme::SPACING.compact);
    Sides::new().truncate().show(
        ui,
        |ui| secondary_text(ui, operation_label(job.operation())),
        |ui| state_text(ui, job.state()),
    );
    if row_action_label(job.state()).is_some() {
        ui.add_space(theme::SPACING.tight);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            show_row_action(ui, job, action)
        });
    }
}

fn show_identity(ui: &mut Ui, job: &QueuedJob) {
    ui.spacing_mut().item_spacing.y = theme::SPACING.tight;
    let source_color = if job.state().is_terminal() {
        theme::SECONDARY
    } else {
        theme::FOREGROUND
    };
    let _ = widgets::truncated_identity(
        ui,
        RichText::new(file_name(&job.media().path))
            .text_style(TextStyle::Body)
            .color(source_color),
        &job.media().path,
    );
    let _ = widgets::truncated_identity(
        ui,
        RichText::new(format!("→ {}", file_name(job.operation().output())))
            .text_style(fonts::technical_text_style())
            .color(theme::SECONDARY_FAINT),
        job.operation().output(),
    );
}

fn show_row_action(ui: &mut Ui, job: &QueuedJob, action: &Cell<QueueAction>) {
    let Some(label) = row_action_label(job.state()) else {
        return;
    };
    if widgets::plain_button(ui, label).clicked() {
        match job.state() {
            QueueJobState::Waiting => action.set(QueueAction::Remove(job.id())),
            QueueJobState::Completed { .. } => action.set(QueueAction::OpenFolder(job.id())),
            _ => {}
        }
    }
}

fn row_action_label(state: &QueueJobState) -> Option<&'static str> {
    match state {
        QueueJobState::Waiting => Some(REMOVE_LABEL),
        QueueJobState::Completed { .. } => Some(OPEN_FOLDER_LABEL),
        _ => None,
    }
}

fn row_action_width(ui: &Ui, jobs: &[QueuedJob]) -> f32 {
    jobs.iter()
        .filter_map(|job| row_action_label(job.state()))
        .map(|label| widgets::plain_button_intrinsic_size(ui, label).x)
        .fold(0.0, f32::max)
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

fn state_text(ui: &mut Ui, state: &QueueJobState) {
    secondary_text(ui, state_label(state));
}

fn failure_details(state: &QueueJobState) -> Option<String> {
    match state {
        QueueJobState::Failed(error) => Some(error.to_string()),
        _ => None,
    }
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

    use super::{
        OPEN_FOLDER_LABEL, QueueSummary, REMOVE_LABEL, bound_active_progress, failure_details,
        file_name, row_action_label, state_label,
    };

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
        assert_eq!(QueueSummary::default().copy(), "0 jobs");
        assert_eq!(
            QueueSummary {
                running: 1,
                waiting: 3,
                finished: 2,
            }
            .copy(),
            "1 running · 3 waiting · 2 finished"
        );
    }

    #[test]
    fn row_filenames_do_not_expand_to_full_paths() {
        assert_eq!(
            file_name(&PathBuf::from("folder/source file.mkv")),
            "source file.mkv"
        );
    }

    #[test]
    fn row_actions_are_owned_only_by_waiting_and_completed_states() {
        assert_eq!(
            row_action_label(&QueueJobState::Waiting),
            Some(REMOVE_LABEL)
        );
        assert_eq!(
            row_action_label(&QueueJobState::Completed {
                output: PathBuf::from("output.mkv")
            }),
            Some(OPEN_FOLDER_LABEL)
        );
        assert_eq!(row_action_label(&QueueJobState::Running), None);
        assert_eq!(
            row_action_label(&QueueJobState::Failed(QueueJobFailure::SourceChanged)),
            None
        );
        assert_eq!(row_action_label(&QueueJobState::Cancelled), None);
    }

    #[test]
    fn failed_details_preserve_the_queue_failure_diagnostic() {
        let failed = QueueJobState::Failed(QueueJobFailure::SourceChanged);
        assert_eq!(
            failure_details(&failed).as_deref(),
            Some("source changed after it was queued")
        );
        assert_eq!(failure_details(&QueueJobState::Waiting), None);
    }
}

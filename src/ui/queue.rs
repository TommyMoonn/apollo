use std::{cell::Cell, path::Path};

use egui::{Align, CollapsingHeader, Label, Layout, RichText, Sides, TextStyle, Ui, vec2};

use crate::{
    media::progress::MediaProgress,
    queue::{JobId, QueueJobState, QueueRunState, QueuedJob, QueuedOperation},
};

use super::{QueueAction, fonts, layout, media_lifecycle, theme, widgets};

const TITLE: &str = "Queue";
const ADD_FILES_LABEL: &str = "Add files";
const EMPTY_COPY: &str = "No jobs queued.";
const START_LABEL: &str = "Start queue";
const CANCEL_LABEL: &str = "Cancel current";
const STOP_AFTER_CURRENT_LABEL: &str = "Stop after current";
const CANCEL_QUEUE_LABEL: &str = "Cancel queue";
const STOPPING_LABEL: &str = "Stopping after current";
const EDIT_LABEL: &str = "Edit";
const MOVE_UP_LABEL: &str = "Move up";
const MOVE_DOWN_LABEL: &str = "Move down";
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

struct QueueRowContext<'a> {
    jobs: &'a [QueuedJob],
    active_job: Option<JobId>,
    active_progress: Option<&'a MediaProgress>,
    action_width: f32,
    run_state: QueueRunState,
    action: &'a Cell<QueueAction>,
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
    run_state: QueueRunState,
    queue_start_allowed: bool,
) -> QueueAction {
    let action = Cell::new(QueueAction::None);
    let capabilities = QueueCapabilities::from_jobs(jobs);
    let summary = QueueSummary::from_jobs(jobs);
    let action_rows = queue_action_rows(ui, active_job, run_state);

    layout::operation_surface_with_action_rows(
        ui,
        action_rows,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            show_header(ui, &summary.copy(), capabilities.has_terminal, &action);
            ui.add_space(theme::SPACING.related);

            if jobs.is_empty() {
                media_lifecycle::secondary_copy(ui, EMPTY_COPY);
            } else {
                let action_width = row_action_width(ui, jobs, run_state);
                let row_context = QueueRowContext {
                    jobs,
                    active_job,
                    active_progress,
                    action_width,
                    run_state,
                    action: &action,
                };
                for (index, job) in jobs.iter().enumerate() {
                    widgets::separator(ui);
                    ui.add_space(theme::SPACING.compact);
                    show_job_row(ui, index, job, &row_context);
                    ui.add_space(theme::SPACING.compact);
                }
                widgets::separator(ui);
            }
        },
        |ui| {
            if action_rows == 1 {
                show_queue_controls(
                    ui,
                    active_job,
                    run_state,
                    capabilities,
                    queue_start_allowed,
                    true,
                    &action,
                );
            } else {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    show_active_controls(ui, run_state, &action);
                });
                ui.add_space(theme::SPACING.compact);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::destructive_button(ui, CANCEL_QUEUE_LABEL).clicked() {
                        action.set(QueueAction::CancelQueue);
                    }
                    if widgets::secondary_button(ui, BACK_LABEL).clicked() {
                        action.set(QueueAction::Back);
                    }
                });
            }
        },
    );

    action.get()
}

fn queue_action_rows(ui: &Ui, active_job: Option<JobId>, run_state: QueueRunState) -> usize {
    if active_job.is_none() {
        return 1;
    }
    let middle_label = match run_state {
        QueueRunState::StoppingAfterCurrent => STOPPING_LABEL,
        QueueRunState::Stopped | QueueRunState::Running => STOP_AFTER_CURRENT_LABEL,
    };
    let widths = [CANCEL_LABEL, middle_label, CANCEL_QUEUE_LABEL, BACK_LABEL]
        .map(|label| widgets::mode_button_intrinsic_size(ui, label).x);
    let content_width = (ui.available_width() - f32::from(layout::CONTENT_PADDING) * 2.0).max(0.0);
    if matches!(
        layout::action_layout(content_width, &widths),
        layout::ActionLayout::Horizontal
    ) {
        1
    } else {
        2
    }
}

fn show_queue_controls(
    ui: &mut Ui,
    active_job: Option<JobId>,
    run_state: QueueRunState,
    capabilities: QueueCapabilities,
    queue_start_allowed: bool,
    show_back: bool,
    action: &Cell<QueueAction>,
) {
    if active_job.is_some() {
        show_active_controls(ui, run_state, action);
        if widgets::destructive_button(ui, CANCEL_QUEUE_LABEL).clicked() {
            action.set(QueueAction::CancelQueue);
        }
    } else if capabilities.has_waiting
        && matches!(run_state, QueueRunState::Stopped)
        && ui
            .add_enabled_ui(queue_start_allowed, |ui| {
                widgets::primary_button(ui, START_LABEL)
            })
            .inner
            .clicked()
    {
        action.set(QueueAction::Start);
    }
    if show_back && widgets::secondary_button(ui, BACK_LABEL).clicked() {
        action.set(QueueAction::Back);
    }
}

fn show_active_controls(ui: &mut Ui, run_state: QueueRunState, action: &Cell<QueueAction>) {
    if widgets::destructive_button(ui, CANCEL_LABEL).clicked() {
        action.set(QueueAction::CancelCurrent);
    }
    match run_state {
        QueueRunState::Running => {
            if widgets::secondary_button(ui, STOP_AFTER_CURRENT_LABEL).clicked() {
                action.set(QueueAction::StopAfterCurrent);
            }
        }
        QueueRunState::StoppingAfterCurrent => {
            media_lifecycle::technical_faint(ui, STOPPING_LABEL);
        }
        QueueRunState::Stopped => {}
    }
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
            ui.horizontal(|ui| {
                if can_clear && widgets::plain_button(ui, CLEAR_LABEL).clicked() {
                    action.set(QueueAction::ClearFinished);
                }
                if widgets::plain_button(ui, ADD_FILES_LABEL).clicked() {
                    action.set(QueueAction::AddFiles);
                }
            });
        },
    );
}

fn show_job_row(ui: &mut Ui, job_index: usize, job: &QueuedJob, context: &QueueRowContext<'_>) {
    let plan = layout::queue_row_plan(ui.available_width(), context.action_width);
    match plan.layout {
        layout::QueueRowLayout::Inline => show_inline_job_row(
            ui,
            context.jobs,
            job_index,
            job,
            plan,
            context.run_state,
            context.action,
        ),
        layout::QueueRowLayout::Stacked => show_stacked_job_row(
            ui,
            context.jobs,
            job_index,
            job,
            context.run_state,
            context.action,
        ),
    }

    if let Some(progress) = bound_active_progress(
        job.id(),
        job.state(),
        context.active_job,
        context.active_progress,
    ) {
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
    jobs: &[QueuedJob],
    job_index: usize,
    job: &QueuedJob,
    plan: layout::QueueRowPlan,
    run_state: QueueRunState,
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
                |ui| show_row_actions(ui, jobs, job_index, job, run_state, action),
            );
        }
    });
}

fn show_stacked_job_row(
    ui: &mut Ui,
    jobs: &[QueuedJob],
    job_index: usize,
    job: &QueuedJob,
    run_state: QueueRunState,
    action: &Cell<QueueAction>,
) {
    show_identity(ui, job);
    ui.add_space(theme::SPACING.compact);
    Sides::new().truncate().show(
        ui,
        |ui| secondary_text(ui, operation_label(job.operation())),
        |ui| state_text(ui, job.state()),
    );
    if has_row_actions(job.state()) {
        ui.add_space(theme::SPACING.tight);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            show_row_actions(ui, jobs, job_index, job, run_state, action)
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

fn show_row_actions(
    ui: &mut Ui,
    jobs: &[QueuedJob],
    job_index: usize,
    job: &QueuedJob,
    run_state: QueueRunState,
    action: &Cell<QueueAction>,
) {
    match job.state() {
        QueueJobState::Waiting => {
            if widgets::plain_button(ui, REMOVE_LABEL).clicked() {
                action.set(QueueAction::Remove(job.id()));
            }
            if matches!(run_state, QueueRunState::Stopped) {
                let (can_move_earlier, can_move_later) = waiting_move_capabilities(jobs, job_index);
                if ui
                    .add_enabled_ui(can_move_later, |ui| {
                        widgets::plain_button(ui, MOVE_DOWN_LABEL)
                    })
                    .inner
                    .clicked()
                {
                    action.set(QueueAction::MoveLater(job.id()));
                }
                if ui
                    .add_enabled_ui(can_move_earlier, |ui| {
                        widgets::plain_button(ui, MOVE_UP_LABEL)
                    })
                    .inner
                    .clicked()
                {
                    action.set(QueueAction::MoveEarlier(job.id()));
                }
                if widgets::plain_button(ui, EDIT_LABEL).clicked() {
                    action.set(QueueAction::Edit(job.id()));
                }
            }
        }
        QueueJobState::Completed { .. }
            if widgets::plain_button(ui, OPEN_FOLDER_LABEL).clicked() =>
        {
            action.set(QueueAction::OpenFolder(job.id()));
        }
        QueueJobState::Completed { .. } => {}
        _ => {}
    }
}

fn has_row_actions(state: &QueueJobState) -> bool {
    matches!(
        state,
        QueueJobState::Waiting | QueueJobState::Completed { .. }
    )
}

fn row_action_width(ui: &Ui, jobs: &[QueuedJob], run_state: QueueRunState) -> f32 {
    let completed = widgets::plain_button_intrinsic_size(ui, OPEN_FOLDER_LABEL).x;
    let waiting = if matches!(run_state, QueueRunState::Stopped) {
        [EDIT_LABEL, MOVE_UP_LABEL, MOVE_DOWN_LABEL, REMOVE_LABEL]
            .iter()
            .map(|label| widgets::plain_button_intrinsic_size(ui, label).x)
            .sum::<f32>()
            + theme::INLINE_ACTION_GAP * 3.0
    } else {
        widgets::plain_button_intrinsic_size(ui, REMOVE_LABEL).x
    };
    if jobs
        .iter()
        .any(|job| matches!(job.state(), QueueJobState::Waiting))
    {
        completed.max(waiting)
    } else {
        completed
    }
}

fn waiting_move_capabilities(jobs: &[QueuedJob], job_index: usize) -> (bool, bool) {
    let waiting_indices = jobs
        .iter()
        .enumerate()
        .filter_map(|(index, job)| matches!(job.state(), QueueJobState::Waiting).then_some(index))
        .collect::<Vec<_>>();
    let Some(position) = waiting_indices.iter().position(|index| *index == job_index) else {
        return (false, false);
    };
    (position > 0, position + 1 < waiting_indices.len())
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
        queue::{JobId, QueueJobFailure, QueueJobState, QueueRunState},
        ui::{fonts, theme},
    };

    use super::{
        CANCEL_LABEL, CANCEL_QUEUE_LABEL, QueueSummary, STOP_AFTER_CURRENT_LABEL, STOPPING_LABEL,
        bound_active_progress, failure_details, file_name, has_row_actions, show, state_label,
    };

    fn shape_text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.job.text == text => {
                Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
            }
            egui::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| shape_text_rect(shape, text))
            }
            _ => None,
        }
    }

    fn output_text_rect(output: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
        output
            .shapes
            .iter()
            .find_map(|shape| shape_text_rect(&shape.shape, text))
    }

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
        assert!(has_row_actions(&QueueJobState::Waiting));
        assert!(has_row_actions(&QueueJobState::Completed {
            output: PathBuf::from("output.mkv")
        }));
        assert!(!has_row_actions(&QueueJobState::Running));
        assert!(!has_row_actions(&QueueJobState::Failed(
            QueueJobFailure::SourceChanged
        )));
        assert!(!has_row_actions(&QueueJobState::Cancelled));
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

    #[test]
    fn compact_running_controls_reflow_without_renaming_destructive_actions() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.set_width(400.0);
            let _ = show(
                ui,
                &[],
                Some(JobId::new(1)),
                None,
                QueueRunState::Running,
                true,
            );
        });

        let cancel_current = output_text_rect(&output, CANCEL_LABEL).unwrap();
        let cancel_queue = output_text_rect(&output, CANCEL_QUEUE_LABEL).unwrap();
        assert!(output_text_rect(&output, STOP_AFTER_CURRENT_LABEL).is_some());
        assert!(cancel_queue.top() > cancel_current.top());
        output.textures_delta.clear();
    }

    #[test]
    fn stopping_controls_replace_stop_action_with_explicit_state_copy() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.set_width(620.0);
            let _ = show(
                ui,
                &[],
                Some(JobId::new(1)),
                None,
                QueueRunState::StoppingAfterCurrent,
                true,
            );
        });

        assert!(output_text_rect(&output, STOPPING_LABEL).is_some());
        assert!(output_text_rect(&output, STOP_AFTER_CURRENT_LABEL).is_none());
        assert!(output_text_rect(&output, CANCEL_LABEL).is_some());
        assert!(output_text_rect(&output, CANCEL_QUEUE_LABEL).is_some());
        output.textures_delta.clear();
    }
}

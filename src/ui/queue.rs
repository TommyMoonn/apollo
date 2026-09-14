use std::{cell::Cell, path::Path};

use egui::{
    Align, CollapsingHeader, Label, Layout, Rect, RichText, Sense, Sides, TextStyle, Ui, UiBuilder,
    vec2,
};

use crate::{
    media::progress::MediaProgress,
    queue::{JobId, QueueJobState, QueueRunState, QueuedJob},
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
const MOVE_EARLIER_LABEL: &str = "↑";
const MOVE_LATER_LABEL: &str = "↓";
const MOVE_EARLIER_HELP: &str = "Move earlier";
const MOVE_LATER_HELP: &str = "Move later";
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
    completed: usize,
    failed: usize,
    cancelled: usize,
}

struct QueueRowContext<'a> {
    move_capabilities: &'a [WaitingMoveCapability],
    active_job: Option<JobId>,
    active_progress: Option<&'a MediaProgress>,
    plan: layout::QueueRowPlan,
    run_state: QueueRunState,
    action: &'a Cell<QueueAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct WaitingMoveCapability {
    earlier: bool,
    later: bool,
}

impl QueueSummary {
    fn from_jobs(jobs: &[QueuedJob]) -> Self {
        let mut summary = Self::default();
        for job in jobs {
            match job.state() {
                QueueJobState::Waiting => summary.waiting += 1,
                QueueJobState::Running => summary.running += 1,
                QueueJobState::Completed { .. } => summary.completed += 1,
                QueueJobState::Failed(_) => summary.failed += 1,
                QueueJobState::Cancelled => summary.cancelled += 1,
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
        if self.completed > 0 {
            parts.push(format!("{} completed", self.completed));
        }
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        if self.cancelled > 0 {
            parts.push(format!("{} cancelled", self.cancelled));
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
    let action_width = (!jobs.is_empty()).then(|| row_action_width(ui, jobs, run_state));

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
                let action_width = action_width.expect("non-empty queue has an action width");
                let row_plan = layout::queue_row_plan(ui.available_width(), action_width);
                let move_capabilities = waiting_move_capabilities(jobs);
                let row_context = QueueRowContext {
                    move_capabilities: &move_capabilities,
                    active_job,
                    active_progress,
                    plan: row_plan,
                    run_state,
                    action: &action,
                };
                if matches!(row_plan.layout, layout::QueueRowLayout::Inline) {
                    show_column_guide(ui, row_plan);
                    ui.add_space(theme::SPACING.tight);
                }
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
                show_queue_footer(
                    ui,
                    active_job,
                    run_state,
                    capabilities,
                    queue_start_allowed,
                    &action,
                );
            } else {
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), theme::PRIMARY_CONTROL_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        if widgets::secondary_button(ui, BACK_LABEL).clicked() {
                            action.set(QueueAction::Back);
                        }
                    },
                );
                ui.add_space(theme::SPACING.compact);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::destructive_button(ui, CANCEL_QUEUE_LABEL).clicked() {
                        action.set(QueueAction::CancelQueue);
                    }
                    show_active_controls(ui, run_state, &action);
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

fn show_queue_footer(
    ui: &mut Ui,
    active_job: Option<JobId>,
    run_state: QueueRunState,
    capabilities: QueueCapabilities,
    queue_start_allowed: bool,
    action: &Cell<QueueAction>,
) {
    Sides::new().show(
        ui,
        |ui| {
            if widgets::secondary_button(ui, BACK_LABEL).clicked() {
                action.set(QueueAction::Back);
            }
        },
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::FOOTER_ACTION_GAP;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if active_job.is_some() {
                    if widgets::destructive_button(ui, CANCEL_QUEUE_LABEL).clicked() {
                        action.set(QueueAction::CancelQueue);
                    }
                    show_active_controls(ui, run_state, action);
                } else if capabilities.has_waiting
                    && matches!(run_state, QueueRunState::Stopped)
                    && widgets::primary_button_enabled(ui, START_LABEL, queue_start_allowed)
                        .clicked()
                {
                    action.set(QueueAction::Start);
                }
            });
        },
    );
}

fn show_active_controls(ui: &mut Ui, run_state: QueueRunState, action: &Cell<QueueAction>) {
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
    if widgets::destructive_button(ui, CANCEL_LABEL).clicked() {
        action.set(QueueAction::CancelCurrent);
    }
}

fn show_header(ui: &mut Ui, summary: &str, can_clear: bool, action: &Cell<QueueAction>) {
    layout::collection_header(
        ui,
        |ui| {
            let _ = layout::collection_heading(ui, TITLE, summary);
        },
        |ui| {
            if widgets::compact_secondary_button(ui, ADD_FILES_LABEL).clicked() {
                action.set(QueueAction::AddFiles);
            }
            if can_clear && widgets::plain_button(ui, CLEAR_LABEL).clicked() {
                action.set(QueueAction::ClearFinished);
            }
        },
    );
}

fn show_column_guide(ui: &mut Ui, plan: layout::QueueRowPlan) {
    let height = ui.text_style_height(&fonts::technical_text_style());
    let cells = allocate_inline_row(ui, plan, height);
    exact_cell(
        ui,
        cells.identity,
        Layout::left_to_right(Align::Center),
        |ui| media_lifecycle::technical_faint(ui, "Source / output"),
    );
    exact_cell(
        ui,
        cells.operation,
        Layout::left_to_right(Align::Center),
        |ui| media_lifecycle::technical_faint(ui, "Operation"),
    );
    exact_cell(
        ui,
        cells.state,
        Layout::left_to_right(Align::Center),
        |ui| media_lifecycle::technical_faint(ui, "Status"),
    );
    if let Some(actions) = cells.actions {
        exact_cell(ui, actions, Layout::right_to_left(Align::Center), |ui| {
            media_lifecycle::technical_faint(ui, "Actions")
        });
    }
}

fn allocate_inline_row(
    ui: &mut Ui,
    plan: layout::QueueRowPlan,
    height: f32,
) -> layout::QueueRowColumns {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    plan.columns(row)
}

fn exact_cell(ui: &mut Ui, rect: Rect, cell_layout: Layout, add_contents: impl FnOnce(&mut Ui)) {
    let mut cell = ui.new_child(UiBuilder::new().max_rect(rect).layout(cell_layout));
    cell.set_min_size(rect.size());
    cell.set_clip_rect(ui.clip_rect().intersect(rect));
    add_contents(&mut cell);
}

fn show_job_row(ui: &mut Ui, job_index: usize, job: &QueuedJob, context: &QueueRowContext<'_>) {
    let plan = context.plan;
    match plan.layout {
        layout::QueueRowLayout::Inline => show_inline_job_row(
            ui,
            context.move_capabilities[job_index],
            job,
            plan,
            context.run_state,
            context.action,
        ),
        layout::QueueRowLayout::Stacked => show_stacked_job_row(
            ui,
            context.move_capabilities[job_index],
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
        ui.add_space(theme::SPACING.related);
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
    move_capability: WaitingMoveCapability,
    job: &QueuedJob,
    plan: layout::QueueRowPlan,
    run_state: QueueRunState,
    action: &Cell<QueueAction>,
) {
    let cells = allocate_inline_row(ui, plan, plan.row_height);
    exact_cell(
        ui,
        cells.identity,
        Layout::top_down(Align::Min).with_main_align(Align::Center),
        |ui| show_identity(ui, job),
    );
    exact_cell(
        ui,
        cells.operation,
        Layout::left_to_right(Align::Center),
        |ui| secondary_text(ui, job.operation().display_name()),
    );
    exact_cell(
        ui,
        cells.state,
        Layout::left_to_right(Align::Center),
        |ui| state_text(ui, job.state()),
    );
    if let Some(actions) = cells.actions {
        exact_cell(ui, actions, Layout::right_to_left(Align::Center), |ui| {
            show_row_actions(ui, move_capability, job, run_state, action)
        });
    }
}

fn show_stacked_job_row(
    ui: &mut Ui,
    move_capability: WaitingMoveCapability,
    job: &QueuedJob,
    run_state: QueueRunState,
    action: &Cell<QueueAction>,
) {
    show_identity(ui, job);
    ui.add_space(theme::SPACING.compact);
    let metadata_height = theme::COMPACT_CONTROL_HEIGHT;
    let (metadata, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), metadata_height), Sense::hover());
    exact_cell(ui, metadata, Layout::left_to_right(Align::Center), |ui| {
        Sides::new().truncate().show(
            ui,
            |ui| secondary_text(ui, job.operation().display_name()),
            |ui| state_text(ui, job.state()),
        );
    });
    if has_row_actions(job.state()) {
        ui.add_space(theme::SPACING.tight);
        let (actions, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), theme::COMPACT_CONTROL_HEIGHT),
            Sense::hover(),
        );
        exact_cell(ui, actions, Layout::right_to_left(Align::Center), |ui| {
            show_row_actions(ui, move_capability, job, run_state, action)
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
    move_capability: WaitingMoveCapability,
    job: &QueuedJob,
    run_state: QueueRunState,
    action: &Cell<QueueAction>,
) {
    ui.spacing_mut().item_spacing.x = theme::INLINE_ACTION_GAP;
    match job.state() {
        QueueJobState::Waiting => {
            if widgets::plain_button(ui, REMOVE_LABEL).clicked() {
                action.set(QueueAction::Remove(job.id()));
            }
            if matches!(run_state, QueueRunState::Stopped) {
                if widgets::plain_button(ui, EDIT_LABEL).clicked() {
                    action.set(QueueAction::Edit(job.id()));
                }
                if widgets::compact_secondary_button_enabled(
                    ui,
                    MOVE_LATER_LABEL,
                    move_capability.later,
                )
                .on_hover_text(MOVE_LATER_HELP)
                .clicked()
                {
                    action.set(QueueAction::MoveLater(job.id()));
                }
                if widgets::compact_secondary_button_enabled(
                    ui,
                    MOVE_EARLIER_LABEL,
                    move_capability.earlier,
                )
                .on_hover_text(MOVE_EARLIER_HELP)
                .clicked()
                {
                    action.set(QueueAction::MoveEarlier(job.id()));
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
        widgets::plain_button_intrinsic_size(ui, EDIT_LABEL).x
            + widgets::compact_secondary_button_intrinsic_size(ui, MOVE_EARLIER_LABEL).x
            + widgets::compact_secondary_button_intrinsic_size(ui, MOVE_LATER_LABEL).x
            + widgets::plain_button_intrinsic_size(ui, REMOVE_LABEL).x
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

fn waiting_move_capabilities(jobs: &[QueuedJob]) -> Vec<WaitingMoveCapability> {
    let waiting_count = jobs
        .iter()
        .filter(|job| matches!(job.state(), QueueJobState::Waiting))
        .count();
    let mut seen_waiting = 0;
    jobs.iter()
        .map(|job| {
            if !matches!(job.state(), QueueJobState::Waiting) {
                return WaitingMoveCapability::default();
            }
            let capability = WaitingMoveCapability {
                earlier: seen_waiting > 0,
                later: seen_waiting + 1 < waiting_count,
            };
            seen_waiting += 1;
            capability
        })
        .collect()
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
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use crate::{
        media::{
            container::MediaContainer, intake::SourceIdentity, model::MediaFile,
            progress::MediaProgress,
        },
        queue::{JobId, QueueJobFailure, QueueJobState, QueueRunState, QueuedJob, QueuedOperation},
        remux::model::RemuxRequest,
        ui::{fonts, theme},
    };

    use super::{
        ADD_FILES_LABEL, BACK_LABEL, CANCEL_LABEL, CANCEL_QUEUE_LABEL, MOVE_EARLIER_HELP,
        MOVE_EARLIER_LABEL, MOVE_LATER_HELP, MOVE_LATER_LABEL, QueueSummary, REMOVE_LABEL,
        STOP_AFTER_CURRENT_LABEL, STOPPING_LABEL, bound_active_progress, failure_details,
        file_name, has_row_actions, show, state_label, waiting_move_capabilities,
    };

    static NEXT_QUEUE_UI_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn waiting_jobs(count: u64) -> (PathBuf, Vec<QueuedJob>) {
        let directory = std::env::temp_dir().join(format!(
            "apollo-queue-ui-{}-{}",
            std::process::id(),
            NEXT_QUEUE_UI_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("nice portal.mp4");
        fs::write(&source, b"media").unwrap();
        let identity = SourceIdentity::read(&source).unwrap();
        let media = MediaFile {
            path: source.clone(),
            duration: None,
            streams: vec![],
            video_streams: vec![],
            audio_streams: vec![],
        };
        let jobs = (1..=count)
            .map(|id| {
                let output = directory.join(format!("nice portal-{id}.mkv"));
                QueuedJob::new(
                    JobId::new(id),
                    identity.clone(),
                    media.clone(),
                    QueuedOperation::Remux(RemuxRequest {
                        input: source.clone(),
                        output: output.clone(),
                        container: MediaContainer::Matroska,
                    }),
                    output,
                )
            })
            .collect();
        (directory, jobs)
    }

    fn waiting_jobs_with_names(names: &[&str]) -> (PathBuf, Vec<QueuedJob>) {
        let directory = std::env::temp_dir().join(format!(
            "apollo-queue-ui-{}-{}",
            std::process::id(),
            NEXT_QUEUE_UI_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let jobs = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let source = directory.join(name);
                fs::write(&source, b"media").unwrap();
                let identity = SourceIdentity::read(&source).unwrap();
                let output = directory.join(format!(
                    "{}-remuxed-with-an-output-name-that-varies-{index}.mkv",
                    name.trim_end_matches(".mp4")
                ));
                let media = MediaFile {
                    path: source.clone(),
                    duration: None,
                    streams: vec![],
                    video_streams: vec![],
                    audio_streams: vec![],
                };
                QueuedJob::new(
                    JobId::new(index as u64 + 1),
                    identity,
                    media,
                    QueuedOperation::Remux(RemuxRequest {
                        input: source,
                        output: output.clone(),
                        container: MediaContainer::Matroska,
                    }),
                    output,
                )
            })
            .collect();
        (directory, jobs)
    }

    fn shape_text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        let mut pending = vec![shape];
        while let Some(shape) = pending.pop() {
            match shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => {
                    return Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()));
                }
                egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
                _ => {}
            }
        }
        None
    }

    fn output_text_rect(output: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
        output
            .shapes
            .iter()
            .find_map(|shape| shape_text_rect(&shape.shape, text))
    }

    fn output_text_rects(output: &egui::FullOutput, text: &str) -> Vec<egui::Rect> {
        let mut matches = Vec::new();
        for clipped in &output.shapes {
            let mut pending = vec![&clipped.shape];
            while let Some(shape) = pending.pop() {
                match shape {
                    egui::Shape::Text(shape) if shape.galley.job.text == text => {
                        matches.push(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
                    }
                    egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
                    _ => {}
                }
            }
        }
        matches.sort_by(|left, right| left.top().total_cmp(&right.top()));
        matches
    }

    fn output_visual_text_rects(output: &egui::FullOutput, text: &str) -> Vec<egui::Rect> {
        let mut matches = Vec::new();
        for clipped in &output.shapes {
            let mut pending = vec![&clipped.shape];
            while let Some(shape) = pending.pop() {
                match shape {
                    egui::Shape::Text(shape) if shape.galley.job.text == text => {
                        matches.push(shape.visual_bounding_rect())
                    }
                    egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
                    _ => {}
                }
            }
        }
        matches.sort_by(|left, right| left.top().total_cmp(&right.top()));
        matches
    }

    fn output_text_baselines(output: &egui::FullOutput, text: &str) -> Vec<f32> {
        let mut matches = Vec::new();
        for clipped in &output.shapes {
            let mut pending = vec![&clipped.shape];
            while let Some(shape) = pending.pop() {
                match shape {
                    egui::Shape::Text(shape) if shape.galley.job.text == text => {
                        let row = shape.galley.rows.first().unwrap();
                        let glyph = row.glyphs.first().unwrap();
                        matches.push(shape.pos.y + row.pos.y + glyph.pos.y);
                    }
                    egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
                    _ => {}
                }
            }
        }
        matches.sort_by(f32::total_cmp);
        matches
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0,
            "expected {actual} to be within one pixel of {expected}"
        );
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
                completed: 2,
                failed: 1,
                cancelled: 4,
            }
            .copy(),
            "1 running · 3 waiting · 2 completed · 1 failed · 4 cancelled"
        );
        assert_eq!(
            QueueSummary {
                failed: 1,
                ..QueueSummary::default()
            }
            .copy(),
            "1 failed"
        );
    }

    #[test]
    fn queue_summary_classifies_each_job_state_without_a_finished_bucket() {
        let (directory, mut jobs) = waiting_jobs(5);
        jobs[0].set_state(QueueJobState::Running);
        jobs[1].set_state(QueueJobState::Completed {
            output: directory.join("nice portal-2.mkv"),
        });
        jobs[2].set_state(QueueJobState::Failed(QueueJobFailure::SourceChanged));
        jobs[3].set_state(QueueJobState::Cancelled);

        assert_eq!(
            QueueSummary::from_jobs(&jobs).copy(),
            "1 running · 1 waiting · 1 completed · 1 failed · 1 cancelled"
        );
        fs::remove_dir_all(directory).unwrap();
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
    fn waiting_move_capabilities_are_prepared_once_in_job_order() {
        let (directory, mut jobs) = waiting_jobs(5);
        let completed_output = jobs[1].operation().output().to_path_buf();
        jobs[1].set_state(QueueJobState::Completed {
            output: completed_output,
        });
        jobs[3].set_state(QueueJobState::Cancelled);

        let capabilities = waiting_move_capabilities(&jobs);
        assert_eq!(capabilities.len(), jobs.len());
        assert!(!capabilities[0].earlier && capabilities[0].later);
        assert_eq!(capabilities[1], Default::default());
        assert!(capabilities[2].earlier && capabilities[2].later);
        assert_eq!(capabilities[3], Default::default());
        assert!(capabilities[4].earlier && !capabilities[4].later);
        fs::remove_dir_all(directory).unwrap();
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
    fn wide_queue_aligns_header_and_exposes_column_guide_and_compact_reorder_controls() {
        let (directory, jobs) = waiting_jobs(2);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.set_width(760.0);
            let _ = show(ui, &jobs, None, None, QueueRunState::Stopped, true);
        });

        let title_baseline = output_text_baselines(&output, "Queue")[0];
        let summary_baseline = output_text_baselines(&output, "2 waiting")[0];
        assert!(output_text_rect(&output, ADD_FILES_LABEL).is_some());
        let identity_guide = output_text_rect(&output, "Source / output").unwrap();
        let operation_guide = output_text_rect(&output, "Operation").unwrap();
        let status_guide = output_text_rect(&output, "Status").unwrap();
        let actions_guide = output_visual_text_rects(&output, "Actions")
            .into_iter()
            .next()
            .unwrap();
        output.textures_delta.clear();

        assert!(
            (title_baseline - summary_baseline).abs() <= 0.25,
            "title baseline={title_baseline}, summary baseline={summary_baseline}"
        );
        assert!(identity_guide.left() < operation_guide.left());
        assert!(operation_guide.left() < status_guide.left());
        assert!(status_guide.left() < actions_guide.left());
        assert!(output_text_rect(&output, MOVE_EARLIER_LABEL).is_some());
        assert!(output_text_rect(&output, MOVE_LATER_LABEL).is_some());
        assert_eq!(MOVE_EARLIER_HELP, "Move earlier");
        assert_eq!(MOVE_LATER_HELP, "Move later");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn default_width_queue_rows_share_exact_guide_anchors_and_truncate_identity() {
        let long_source =
            "an-extraordinarily-long-source-filename-that-must-not-move-later-columns.mp4";
        let long_output = format!(
            "→ {}-remuxed-with-an-output-name-that-varies-2.mkv",
            long_source.trim_end_matches(".mp4")
        );
        let (directory, jobs) =
            waiting_jobs_with_names(&["a.mp4", "a-medium-length-source-filename.mp4", long_source]);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut full_source_width = 0.0;
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.set_width(727.0);
            full_source_width = ui
                .painter()
                .layout_no_wrap(
                    long_source.to_owned(),
                    egui::TextStyle::Body.resolve(ui.style()),
                    theme::FOREGROUND,
                )
                .size()
                .x;
            let _ = show(ui, &jobs, None, None, QueueRunState::Stopped, true);
        });

        let operation_guide = output_text_rect(&output, "Operation").unwrap();
        let status_guide = output_text_rect(&output, "Status").unwrap();
        let actions_guide = output_text_rect(&output, "Actions").unwrap();
        let operation_cells = output_text_rects(&output, "Remux");
        let status_cells = output_text_rects(&output, "Waiting");
        let action_cells = output_visual_text_rects(&output, REMOVE_LABEL);
        let source = output_text_rect(&output, long_source).unwrap();
        let output_identity = output_text_rect(&output, &long_output).unwrap();
        output.textures_delta.clear();

        assert_eq!(operation_cells.len(), jobs.len());
        assert_eq!(status_cells.len(), jobs.len());
        assert_eq!(action_cells.len(), jobs.len());
        for operation in operation_cells {
            assert_near(operation.left(), operation_guide.left());
        }
        for status in status_cells {
            assert_near(status.left(), status_guide.left());
        }
        let action_anchor = action_cells[0].right();
        for actions in action_cells {
            assert_near(actions.right(), action_anchor);
        }
        assert!(actions_guide.left() > status_guide.right());
        assert!(action_anchor > status_guide.right());
        assert!(source.right() < operation_guide.left());
        assert!(output_identity.right() < operation_guide.left());
        assert!(source.width() < full_source_width);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn narrow_queue_keeps_stacked_rows_without_a_column_guide() {
        let (directory, jobs) = waiting_jobs(1);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.set_width(400.0);
            let _ = show(ui, &jobs, None, None, QueueRunState::Stopped, true);
        });

        assert!(output_text_rect(&output, "Source / output").is_none());
        let source = output_text_rect(&output, "nice portal.mp4").unwrap();
        let operation = output_text_rect(&output, "Remux").unwrap();
        let status = output_text_rect(&output, "Waiting").unwrap();
        let remove = output_text_rect(&output, REMOVE_LABEL).unwrap();
        output.textures_delta.clear();

        assert!(operation.top() > source.top());
        assert_near(operation.center().y, status.center().y);
        assert!(remove.top() - operation.bottom() < theme::COMPACT_CONTROL_HEIGHT);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn wide_queue_footer_separates_back_from_right_owned_process_controls() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.set_width(760.0);
            let _ = show(
                ui,
                &[],
                Some(JobId::new(1)),
                None,
                QueueRunState::Running,
                true,
            );
        });
        let back = output_text_rect(&output, BACK_LABEL).unwrap();
        let cancel_current = output_text_rect(&output, CANCEL_LABEL).unwrap();
        let stop = output_text_rect(&output, STOP_AFTER_CURRENT_LABEL).unwrap();
        let cancel_queue = output_text_rect(&output, CANCEL_QUEUE_LABEL).unwrap();
        output.textures_delta.clear();

        assert_eq!(back.top(), cancel_current.top());
        assert_eq!(cancel_current.top(), stop.top());
        assert_eq!(stop.top(), cancel_queue.top());
        assert!(back.left() < cancel_current.left());
        assert!(cancel_current.left() < stop.left());
        assert!(stop.left() < cancel_queue.left());
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
        let stop = output_text_rect(&output, STOP_AFTER_CURRENT_LABEL).unwrap();
        let back = output_text_rect(&output, BACK_LABEL).unwrap();
        output.textures_delta.clear();
        assert!(cancel_current.top() > back.top());
        assert_eq!(cancel_queue.top(), cancel_current.top());
        assert_eq!(stop.top(), cancel_current.top());
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

use std::{cell::Cell, path::Path};

use egui::{Align, Layout, Rect, RichText, Sense, TextStyle, Ui, UiBuilder, vec2};

use crate::media::error::MediaError;

use super::{fonts, layout, media_error, media_lifecycle, theme, widgets, workspace::Operation};

const TITLE: &str = "Batch setup";
const DEFAULTS_LABEL: &str = "Configure defaults";
const ADD_FILES_LABEL: &str = "Add files";
const OVERRIDE_LABEL: &str = "Override";
const REVIEW_LABEL: &str = "Review";
const EXCLUDE_LABEL: &str = "Exclude";
const INCLUDE_LABEL: &str = "Include";
const ENQUEUE_LABEL: &str = "Add batch to queue";
const BACK_LABEL: &str = "Back";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchReadiness {
    Inspecting,
    Ready,
    NeedsAttention,
    Incompatible,
}

impl BatchReadiness {
    const fn label(self) -> &'static str {
        match self {
            Self::Inspecting => "Inspecting",
            Self::Ready => "Ready",
            Self::NeedsAttention => "Needs attention",
            Self::Incompatible => "Incompatible",
        }
    }
}

pub struct BatchItemView<'a> {
    pub source: &'a Path,
    pub output: Option<&'a Path>,
    pub readiness: BatchReadiness,
    pub reason: Option<&'a str>,
    pub included: bool,
    pub edit_action: Option<BatchItemEditAction>,
    pub can_toggle_included: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchItemEditAction {
    Override,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatchAction {
    SelectOperation(Operation),
    ConfigureDefaults,
    AddFiles,
    EditItem(usize),
    ToggleIncluded(usize),
    Enqueue,
    Back,
    #[default]
    None,
}

pub struct BatchFeedback<'a> {
    pub status: Option<&'a str>,
    pub append_error: Option<&'a MediaError>,
    pub append_error_details_open: &'a mut bool,
}

pub fn show(
    ui: &mut Ui,
    operation: Operation,
    items: &[BatchItemView<'_>],
    can_configure: bool,
    can_add_files: bool,
    can_enqueue: bool,
    feedback: BatchFeedback<'_>,
) -> BatchAction {
    let action = Cell::new(BatchAction::None);
    layout::operation_surface(
        ui,
        |ui| {
            show_header(ui, items, can_configure, can_add_files, &action);
            ui.add_space(theme::SPACING.related);
            show_operation_switcher(ui, operation, &action);
            ui.add_space(theme::SPACING.related);
            let action_width = batch_action_width(ui);
            let row_plan = layout::batch_row_plan(ui.available_width(), action_width);
            if matches!(row_plan.layout, layout::BatchRowLayout::Inline) {
                show_column_guide(ui, row_plan);
                ui.add_space(theme::SPACING.tight);
            }
            for (index, item) in items.iter().enumerate() {
                widgets::separator(ui);
                ui.add_space(theme::SPACING.compact);
                show_item(ui, index, item, row_plan, &action);
                ui.add_space(theme::SPACING.compact);
            }
            widgets::separator(ui);
            if let Some(error) = feedback.status {
                ui.add_space(theme::SPACING.related);
                let _ = widgets::inline_status(ui, error, widgets::InlineStatusKind::Error);
            }
            if let Some(error) = feedback.append_error {
                ui.add_space(theme::SPACING.related);
                media_error::show_inline(ui, error, feedback.append_error_details_open);
            }
        },
        |ui| {
            if widgets::primary_button_enabled(ui, ENQUEUE_LABEL, can_enqueue).clicked() {
                action.set(BatchAction::Enqueue);
            }
            if widgets::secondary_button(ui, BACK_LABEL).clicked() {
                action.set(BatchAction::Back);
            }
        },
    );
    action.get()
}

fn show_header(
    ui: &mut Ui,
    items: &[BatchItemView<'_>],
    can_configure: bool,
    can_add_files: bool,
    action: &Cell<BatchAction>,
) {
    let included = items.iter().filter(|item| item.included).count();
    let summary = format!("{included} of {} included", items.len());
    let heading_width = ui
        .painter()
        .layout_no_wrap(
            format!("{TITLE} {summary}"),
            TextStyle::Heading.resolve(ui.style()),
            theme::FOREGROUND,
        )
        .size()
        .x;
    let action_width = widgets::compact_secondary_button_intrinsic_size(ui, DEFAULTS_LABEL).x
        + widgets::compact_secondary_button_intrinsic_size(ui, ADD_FILES_LABEL).x
        + theme::SPACING.related;
    if heading_width + action_width + theme::SPACING.related > ui.available_width() {
        let _ = layout::collection_heading(ui, TITLE, &summary);
        ui.add_space(theme::SPACING.compact);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), theme::COMPACT_CONTROL_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| show_header_actions(ui, can_configure, can_add_files, action),
        );
        return;
    }
    layout::collection_header(
        ui,
        |ui| {
            let _ = layout::collection_heading(ui, TITLE, &summary);
        },
        |ui| show_header_actions(ui, can_configure, can_add_files, action),
    );
}

fn show_header_actions(
    ui: &mut Ui,
    can_configure: bool,
    can_add_files: bool,
    action: &Cell<BatchAction>,
) {
    if widgets::compact_secondary_button_enabled(ui, ADD_FILES_LABEL, can_add_files).clicked() {
        action.set(BatchAction::AddFiles);
    }
    if widgets::compact_secondary_button_enabled(ui, DEFAULTS_LABEL, can_configure).clicked() {
        action.set(BatchAction::ConfigureDefaults);
    }
}

fn show_operation_switcher(ui: &mut Ui, active: Operation, action: &Cell<BatchAction>) {
    let widths = Operation::ALL
        .map(|operation| widgets::mode_button_intrinsic_size(ui, operation.label()).x);
    let split = !layout::workspace_switcher_fits_one_line(ui.available_width(), &widths);
    show_operation_row(
        ui,
        if split {
            &Operation::ALL[..3]
        } else {
            &Operation::ALL
        },
        active,
        action,
    );
    if split {
        ui.add_space(theme::SPACING.compact);
        show_operation_row(ui, &Operation::ALL[3..], active, action);
    }
}

fn show_operation_row(
    ui: &mut Ui,
    operations: &[Operation],
    active: Operation,
    action: &Cell<BatchAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACING.related;
        for &operation in operations {
            if widgets::mode_button(ui, operation.label(), operation == active).clicked() {
                action.set(BatchAction::SelectOperation(operation));
            }
        }
    });
}

fn show_item(
    ui: &mut Ui,
    index: usize,
    item: &BatchItemView<'_>,
    plan: layout::BatchRowPlan,
    action: &Cell<BatchAction>,
) {
    match plan.layout {
        layout::BatchRowLayout::Inline => show_inline_item(ui, index, item, plan, action),
        layout::BatchRowLayout::Stacked => show_stacked_item(ui, index, item, action),
    }
}

fn show_inline_item(
    ui: &mut Ui,
    index: usize,
    item: &BatchItemView<'_>,
    plan: layout::BatchRowPlan,
    action: &Cell<BatchAction>,
) {
    let (row, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), plan.row_height), Sense::hover());
    let cells = plan.columns(row);
    exact_cell(
        ui,
        cells.identity,
        Layout::top_down(Align::Min).with_main_align(Align::Center),
        |ui| show_identity(ui, item),
    );
    exact_cell(
        ui,
        cells.state,
        Layout::top_down(Align::Min).with_main_align(Align::Center),
        |ui| show_status(ui, item),
    );
    exact_cell(
        ui,
        cells.actions,
        Layout::right_to_left(Align::Center),
        |ui| show_item_actions(ui, index, item, action),
    );
}

fn show_stacked_item(
    ui: &mut Ui,
    index: usize,
    item: &BatchItemView<'_>,
    action: &Cell<BatchAction>,
) {
    show_identity(ui, item);
    ui.add_space(theme::SPACING.compact);
    show_status(ui, item);
    if item.edit_action.is_some() || item.can_toggle_included {
        ui.add_space(theme::SPACING.tight);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), theme::COMPACT_CONTROL_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| show_item_actions(ui, index, item, action),
        );
    }
}

fn show_identity(ui: &mut Ui, item: &BatchItemView<'_>) {
    let file_name = item
        .source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let output_name = item
        .output
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy());
    ui.spacing_mut().item_spacing.y = theme::SPACING.tight;
    let _ = widgets::truncated_identity(
        ui,
        RichText::new(file_name).text_style(TextStyle::Body),
        item.source,
    );
    if let (Some(output), Some(output_name)) = (item.output, output_name) {
        let _ = widgets::truncated_identity(
            ui,
            RichText::new(format!("→ {output_name}"))
                .text_style(fonts::technical_text_style())
                .color(theme::SECONDARY_FAINT),
            output,
        );
    }
}

fn show_status(ui: &mut Ui, item: &BatchItemView<'_>) {
    ui.spacing_mut().item_spacing.y = theme::SPACING.tight;
    media_lifecycle::technical_faint(ui, item.readiness.label());
    if let Some(reason) = item.reason {
        let response = ui.add(
            egui::Label::new(
                RichText::new(reason)
                    .text_style(fonts::technical_text_style())
                    .color(theme::SECONDARY_FAINT),
            )
            .truncate()
            .selectable(false),
        );
        response.on_hover_text(reason);
    }
}

fn show_item_actions(
    ui: &mut Ui,
    index: usize,
    item: &BatchItemView<'_>,
    action: &Cell<BatchAction>,
) {
    ui.spacing_mut().item_spacing.x = theme::INLINE_ACTION_GAP;
    if item.can_toggle_included {
        let toggle_label = if item.included {
            EXCLUDE_LABEL
        } else {
            INCLUDE_LABEL
        };
        if widgets::plain_button(ui, toggle_label).clicked() {
            action.set(BatchAction::ToggleIncluded(index));
        }
    }
    if let Some(edit_action) = item.edit_action {
        let label = match edit_action {
            BatchItemEditAction::Override => OVERRIDE_LABEL,
            BatchItemEditAction::Review => REVIEW_LABEL,
        };
        if widgets::plain_button(ui, label).clicked() {
            action.set(BatchAction::EditItem(index));
        }
    }
}

fn batch_action_width(ui: &Ui) -> f32 {
    let edit = widgets::plain_button_intrinsic_size(ui, OVERRIDE_LABEL)
        .x
        .max(widgets::plain_button_intrinsic_size(ui, REVIEW_LABEL).x);
    let toggle = widgets::plain_button_intrinsic_size(ui, EXCLUDE_LABEL)
        .x
        .max(widgets::plain_button_intrinsic_size(ui, INCLUDE_LABEL).x);
    edit + theme::INLINE_ACTION_GAP + toggle
}

fn show_column_guide(ui: &mut Ui, plan: layout::BatchRowPlan) {
    let height = ui.text_style_height(&fonts::technical_text_style());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let cells = plan.columns(row);
    exact_cell(
        ui,
        cells.identity,
        Layout::left_to_right(Align::Center),
        |ui| media_lifecycle::technical_faint(ui, "Source / output"),
    );
    exact_cell(
        ui,
        cells.state,
        Layout::left_to_right(Align::Center),
        |ui| media_lifecycle::technical_faint(ui, "Status"),
    );
    exact_cell(
        ui,
        cells.actions,
        Layout::right_to_left(Align::Center),
        |ui| media_lifecycle::technical_faint(ui, "Actions"),
    );
}

fn exact_cell(ui: &mut Ui, rect: Rect, cell_layout: Layout, add_contents: impl FnOnce(&mut Ui)) {
    let mut cell = ui.new_child(UiBuilder::new().max_rect(rect).layout(cell_layout));
    cell.set_min_size(rect.size());
    cell.set_clip_rect(ui.clip_rect().intersect(rect));
    add_contents(&mut cell);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        ADD_FILES_LABEL, BatchFeedback, BatchItemEditAction, BatchItemView, BatchReadiness,
        DEFAULTS_LABEL, TITLE, show,
    };
    use crate::ui::{fonts, theme, workspace::Operation};

    fn output_text_baseline(output: &egui::FullOutput, text: &str) -> Option<f32> {
        for clipped in &output.shapes {
            let mut pending = vec![&clipped.shape];
            while let Some(shape) = pending.pop() {
                match shape {
                    egui::Shape::Text(shape) if shape.galley.job.text == text => {
                        let row = shape.galley.rows.first()?;
                        let glyph = row.glyphs.first()?;
                        return Some(shape.pos.y + row.pos.y + glyph.pos.y);
                    }
                    egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
                    _ => {}
                }
            }
        }
        None
    }

    fn output_text_rects(output: &egui::FullOutput, text: &str) -> Vec<egui::Rect> {
        let mut rects = Vec::new();
        for clipped in &output.shapes {
            let mut pending = vec![&clipped.shape];
            while let Some(shape) = pending.pop() {
                match shape {
                    egui::Shape::Text(shape) if shape.galley.job.text == text => {
                        rects.push(egui::Rect::from_min_size(shape.pos, shape.galley.size()));
                    }
                    egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
                    _ => {}
                }
            }
        }
        rects
    }

    #[test]
    fn readiness_copy_is_explicit() {
        assert_eq!(BatchReadiness::Inspecting.label(), "Inspecting");
        assert_eq!(BatchReadiness::NeedsAttention.label(), "Needs attention");
        assert_eq!(BatchReadiness::Incompatible.label(), "Incompatible");
    }

    #[test]
    fn batch_collection_header_aligns_title_and_summary_on_one_glyph_baseline() {
        let paths = ["first.mp4", "second.mp4", "third.mp4"];
        let items = paths.map(|path| BatchItemView {
            source: Path::new(path),
            output: None,
            readiness: BatchReadiness::Ready,
            reason: None,
            included: true,
            edit_action: Some(BatchItemEditAction::Override),
            can_toggle_included: true,
        });
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.set_width(760.0);
            let _ = show(
                ui,
                Operation::Remux,
                &items,
                true,
                true,
                true,
                BatchFeedback {
                    status: None,
                    append_error: None,
                    append_error_details_open: &mut false,
                },
            );
        });

        let title_baseline = output_text_baseline(&output, TITLE).unwrap();
        let summary_baseline = output_text_baseline(&output, "3 of 3 included").unwrap();
        assert!(output_text_baseline(&output, DEFAULTS_LABEL).is_some());
        assert!(output_text_baseline(&output, ADD_FILES_LABEL).is_some());
        output.textures_delta.clear();

        assert!(
            (title_baseline - summary_baseline).abs() <= 0.25,
            "title baseline={title_baseline}, summary baseline={summary_baseline}"
        );
    }

    #[test]
    fn wide_batch_rows_share_status_and_action_anchors_for_long_identities() {
        let paths = [
            "a.mp4",
            "a-medium-length-source-name.mp4",
            "an-extremely-long-source-name-that-must-truncate-inside-the-flexible-identity-column.mp4",
        ];
        let items = paths.map(|path| BatchItemView {
            source: Path::new(path),
            output: Some(Path::new(
                "equally-long-output-name-that-must-also-truncate.mkv",
            )),
            readiness: BatchReadiness::Ready,
            reason: None,
            included: true,
            edit_action: Some(BatchItemEditAction::Override),
            can_toggle_included: true,
        });
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.set_width(730.0);
            let _ = show(
                ui,
                Operation::Remux,
                &items,
                true,
                true,
                true,
                BatchFeedback {
                    status: None,
                    append_error: None,
                    append_error_details_open: &mut false,
                },
            );
        });

        let ready = output_text_rects(&output, "Ready");
        let exclude = output_text_rects(&output, "Exclude");
        assert_eq!(ready.len(), 3);
        assert_eq!(exclude.len(), 3);
        assert!(
            ready
                .iter()
                .all(|rect| (rect.left() - ready[0].left()).abs() <= 0.5)
        );
        assert!(
            exclude
                .iter()
                .all(|rect| (rect.right() - exclude[0].right()).abs() <= 0.5)
        );
        assert!(output_text_rects(&output, "Source / output").len() == 1);
        assert!(output_text_rects(&output, "Status").len() == 1);
        assert!(output_text_rects(&output, "Actions").len() == 1);
        output.textures_delta.clear();
    }
}

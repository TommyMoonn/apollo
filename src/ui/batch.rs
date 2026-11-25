use std::{cell::Cell, path::Path};

use egui::{Align, Layout, Rect, RichText, Sense, TextStyle, Ui, UiBuilder, vec2};

use crate::{app::Operation, media::error::MediaError};

use super::{fonts, layout, media_error, media_lifecycle, theme, widgets};

const TITLE: &str = "Batch setup";
const DEFAULTS_LABEL: &str = "Edit shared settings";
const ADD_FILES_LABEL: &str = "Add files";
const EDIT_LABEL: &str = "Edit settings";
const SKIP_LABEL: &str = "Skip";
const INCLUDE_LABEL: &str = "Include";
const BACK_LABEL: &str = "Back";

use crate::app::{BatchAction, BatchAvailability, BatchItemId, BatchReadiness};

impl BatchReadiness {
    const fn label(self) -> &'static str {
        match self {
            Self::Inspecting => "Checking",
            Self::Ready => "Ready",
            Self::NeedsAttention => "Needs review",
            Self::Incompatible => "Incompatible",
        }
    }
}

pub struct BatchItemView<'a> {
    pub id: BatchItemId,
    pub source: &'a Path,
    pub output: Option<&'a Path>,
    pub readiness: BatchReadiness,
    pub reason: Option<&'a str>,
    pub included: bool,
    pub can_edit: bool,
    pub overridden: bool,
    pub inspection_failed: bool,
    pub can_toggle_included: bool,
}

impl BatchItemView<'_> {
    fn status(&self) -> &'static str {
        if self.inspection_failed {
            "Couldn't open"
        } else if !self.included {
            "Skipped"
        } else {
            self.readiness.label()
        }
    }
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
    availability: BatchAvailability,
    pending_queue_count: usize,
    queue_enabled: bool,
    feedback: BatchFeedback<'_>,
) -> BatchAction {
    let action = Cell::new(BatchAction::None);
    layout::operation_surface(
        ui,
        |ui| {
            show_header(
                ui,
                items,
                availability.can_configure,
                availability.can_add_files,
                &super::queue_count_label(pending_queue_count),
                queue_enabled,
                &action,
            );
            ui.add_space(theme::SPACING.related);
            show_operation_switcher(ui, operation, &action);
            ui.add_space(theme::SPACING.related);
            let action_width = batch_action_width(ui);
            let metadata_lines = if items.iter().any(|item| item.overridden) {
                2.0
            } else {
                1.0
            };
            let row_height = ui.text_style_height(&TextStyle::Body)
                + (ui.text_style_height(&fonts::technical_text_style()) + theme::SPACING.tight)
                    * metadata_lines;
            let row_plan = layout::batch_row_plan(
                ui.available_width(),
                action_width,
                batch_state_width(ui),
                row_height,
            );
            if matches!(row_plan.layout, layout::BatchRowLayout::Inline) {
                show_column_guide(ui, row_plan);
                ui.add_space(theme::SPACING.tight);
            }
            for item in items {
                widgets::separator(ui);
                ui.add_space(theme::SPACING.compact);
                ui.push_id(item.id.0, |ui| {
                    show_item(ui, item.id, item, row_plan, &action)
                });
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
            let included = items.iter().filter(|item| item.included).count();
            let label = if included == 1 {
                "Add 1 job to queue".to_owned()
            } else {
                format!("Add {included} jobs to queue")
            };
            if widgets::primary_button_enabled(ui, &label, availability.can_enqueue).clicked() {
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
    queue_label: &str,
    queue_enabled: bool,
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
        + widgets::compact_secondary_button_intrinsic_size(ui, queue_label).x
        + theme::SPACING.related * 2.0;
    if heading_width + action_width + theme::SPACING.related > ui.available_width() {
        let _ = layout::collection_heading(ui, TITLE, &summary);
        ui.add_space(theme::SPACING.compact);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), theme::COMPACT_CONTROL_HEIGHT),
            Layout::right_to_left(Align::Center).with_main_wrap(true),
            |ui| {
                show_header_actions(
                    ui,
                    can_configure,
                    can_add_files,
                    queue_label,
                    queue_enabled,
                    action,
                )
            },
        );
        return;
    }
    layout::collection_header(
        ui,
        |ui| {
            let _ = layout::collection_heading(ui, TITLE, &summary);
        },
        |ui| {
            show_header_actions(
                ui,
                can_configure,
                can_add_files,
                queue_label,
                queue_enabled,
                action,
            )
        },
    );
}

fn show_header_actions(
    ui: &mut Ui,
    can_configure: bool,
    can_add_files: bool,
    queue_label: &str,
    queue_enabled: bool,
    action: &Cell<BatchAction>,
) {
    if widgets::compact_secondary_button_enabled(ui, queue_label, queue_enabled).clicked() {
        action.set(BatchAction::OpenQueue);
    }
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
    index: BatchItemId,
    item: &BatchItemView<'_>,
    plan: layout::BatchRowPlan,
    action: &Cell<BatchAction>,
) {
    match plan.layout {
        layout::BatchRowLayout::Inline => show_inline_item(ui, index, item, plan, action),
        layout::BatchRowLayout::Stacked => show_stacked_item(ui, index, item, plan, action),
    }
}

fn show_inline_item(
    ui: &mut Ui,
    index: BatchItemId,
    item: &BatchItemView<'_>,
    plan: layout::BatchRowPlan,
    action: &Cell<BatchAction>,
) {
    let (row, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), plan.row_height), Sense::hover());
    let cells = plan.columns(row);
    show_identity_cell(ui, cells.identity, item);
    exact_cell(
        ui,
        cells.state,
        Layout::left_to_right(Align::Center),
        |ui| show_status(ui, item),
    );
    show_action_cell(ui, cells.actions, index, item, action);
}

fn show_stacked_item(
    ui: &mut Ui,
    index: BatchItemId,
    item: &BatchItemView<'_>,
    plan: layout::BatchRowPlan,
    action: &Cell<BatchAction>,
) {
    let (identity, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), plan.row_height), Sense::hover());
    show_identity_cell(ui, identity, item);
    ui.add_space(theme::SPACING.compact);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), theme::COMPACT_CONTROL_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| show_status(ui, item),
    );
    ui.add_space(theme::SPACING.tight);
    let (edit, toggle) = action_sizes(ui);
    let (row, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), edit.y.max(toggle.y)),
        Sense::hover(),
    );
    let group = Rect::from_min_max(
        egui::pos2(row.right() - plan.action_width, row.top()),
        row.right_bottom(),
    );
    show_action_cell(ui, group, index, item, action);
}

fn show_identity_cell(ui: &mut Ui, rect: Rect, item: &BatchItemView<'_>) {
    let metadata_lines =
        usize::from(item.output.and_then(Path::file_name).is_some()) + usize::from(item.overridden);
    let height = ui.text_style_height(&TextStyle::Body)
        + (ui.text_style_height(&fonts::technical_text_style()) + theme::SPACING.tight)
            * metadata_lines as f32;
    let identity =
        Rect::from_center_size(rect.center(), vec2(rect.width(), height.min(rect.height())));
    exact_cell(ui, identity, Layout::top_down(Align::Min), |ui| {
        show_identity(ui, item)
    });
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
    if item.overridden {
        ui.add(
            egui::Label::new(
                RichText::new("Custom settings")
                    .text_style(fonts::technical_text_style())
                    .color(theme::SECONDARY_FAINT),
            )
            .extend()
            .selectable(false),
        );
    }
}

fn show_status(ui: &mut Ui, item: &BatchItemView<'_>) {
    let status = RichText::new(item.status())
        .text_style(TextStyle::Button)
        .color(theme::SECONDARY);
    let Some(reason) = item.reason.filter(|reason| !reason.is_empty()) else {
        ui.add(egui::Label::new(status).extend().selectable(false));
        return;
    };

    let id = ui
        .id()
        .with(("batch-reason", item.id.0, item.source, reason));
    // The status is a local diagnostic link, not a navigation or browser action.
    let response = ui
        .scope(|ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.visuals_mut().hyperlink_color = theme::SECONDARY;
            ui.link(status)
        })
        .inner;
    if response.clicked() {
        ui.data_mut(|data| data.insert_temp(id, true));
    }
    if ui.data(|data| data.get_temp::<bool>(id).unwrap_or(false)) {
        let modal = egui::Modal::new(id).show(ui.ctx(), |ui| {
            ui.set_max_width(400.0);
            ui.heading(item.status());
            media_lifecycle::diagnostic_viewport(ui, reason);
            widgets::secondary_button(ui, "Close").clicked()
        });
        if modal.inner || modal.should_close() {
            ui.data_mut(|data| data.remove::<bool>(id));
        }
    }
}

fn action_sizes(ui: &Ui) -> (egui::Vec2, egui::Vec2) {
    let edit = widgets::plain_action_slot_size(ui, EDIT_LABEL);
    let toggle = widgets::plain_action_slot_size(ui, SKIP_LABEL)
        .max(widgets::plain_action_slot_size(ui, INCLUDE_LABEL));
    (edit, toggle)
}

// Allocate the complete action pair from left to right so keyboard order follows visual order.
fn show_action_cell(
    ui: &mut Ui,
    rect: Rect,
    index: BatchItemId,
    item: &BatchItemView<'_>,
    action: &Cell<BatchAction>,
) {
    let mut cell = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    cell.set_min_size(rect.size());
    let focus_outset = theme::FOCUS_OFFSET + theme::FOCUS_STROKE_WIDTH;
    cell.set_clip_rect(ui.clip_rect().intersect(rect.expand(focus_outset)));
    show_item_actions(&mut cell, index, item, action);
}

fn show_item_actions(
    ui: &mut Ui,
    index: BatchItemId,
    item: &BatchItemView<'_>,
    action: &Cell<BatchAction>,
) {
    ui.spacing_mut().item_spacing.x = theme::INLINE_ACTION_GAP;
    let (edit, toggle) = action_sizes(ui);
    if item.can_edit {
        if widgets::plain_button_sized(ui, EDIT_LABEL, edit).clicked() {
            action.set(BatchAction::EditItem(index));
        }
    } else {
        ui.allocate_space(edit);
    }
    if item.can_toggle_included {
        let label = if item.included {
            SKIP_LABEL
        } else {
            INCLUDE_LABEL
        };
        if widgets::plain_button_sized(ui, label, toggle).clicked() {
            action.set(BatchAction::ToggleIncluded(index));
        }
    } else {
        ui.allocate_space(toggle);
    }
}

fn batch_action_width(ui: &Ui) -> f32 {
    let (edit, toggle) = action_sizes(ui);
    (edit.x + theme::INLINE_ACTION_GAP + toggle.x).ceil()
}

fn batch_state_width(ui: &Ui) -> f32 {
    let width = [
        "Checking",
        "Ready",
        "Needs review",
        "Incompatible",
        "Skipped",
        "Couldn't open",
    ]
    .into_iter()
    .map(|label| {
        ui.painter()
            .layout_no_wrap(
                label.to_owned(),
                TextStyle::Button.resolve(ui.style()),
                theme::SECONDARY,
            )
            .size()
            .x
    })
    .fold(0.0, f32::max);
    width.ceil()
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
        ADD_FILES_LABEL, BatchFeedback, BatchItemView, BatchReadiness, DEFAULTS_LABEL, TITLE, show,
    };
    use crate::{
        app::Operation,
        ui::{fonts, theme},
    };

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
    fn batch_collection_header_aligns_title_and_summary_on_one_glyph_baseline() {
        let paths = ["first.mp4", "second.mp4", "third.mp4"];
        let mut ids = 0..;
        let items = paths.map(|path| BatchItemView {
            id: crate::app::batch::BatchItemId(ids.next().unwrap()),
            source: Path::new(path),
            output: None,
            readiness: BatchReadiness::Ready,
            reason: None,
            included: true,
            can_edit: true,
            overridden: false,
            inspection_failed: false,
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
                crate::app::BatchAvailability {
                    can_configure: true,
                    can_add_files: true,
                    can_enqueue: true,
                },
                0,
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
            (title_baseline - summary_baseline).abs() <= 1.0,
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
        let mut ids = 0..;
        let items = paths.map(|path| BatchItemView {
            id: crate::app::batch::BatchItemId(ids.next().unwrap()),
            source: Path::new(path),
            output: Some(Path::new(
                "equally-long-output-name-that-must-also-truncate.mkv",
            )),
            readiness: BatchReadiness::Ready,
            reason: None,
            included: true,
            can_edit: true,
            overridden: false,
            inspection_failed: false,
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
                crate::app::BatchAvailability {
                    can_configure: true,
                    can_add_files: true,
                    can_enqueue: true,
                },
                0,
                true,
                BatchFeedback {
                    status: None,
                    append_error: None,
                    append_error_details_open: &mut false,
                },
            );
        });

        let ready = output_text_rects(&output, "Ready");
        let exclude = output_text_rects(&output, "Skip");
        assert_eq!(ready.len(), 3);
        assert_eq!(exclude.len(), 3);
        assert!(
            ready
                .iter()
                .all(|rect| (rect.left() - ready[0].left()).abs() <= 1.0)
        );
        assert!(
            exclude
                .iter()
                .all(|rect| (rect.right() - exclude[0].right()).abs() <= 1.0)
        );
        assert!(output_text_rects(&output, "Source / output").len() == 1);
        assert!(output_text_rects(&output, "Status").len() == 1);
        assert!(output_text_rects(&output, "Actions").len() == 1);
        output.textures_delta.clear();
    }

    #[test]
    fn diagnostic_status_opens_reason_on_click_without_a_details_action() {
        let reason = "No audio tracks.";
        let item = BatchItemView {
            id: crate::app::BatchItemId(0),
            source: Path::new("silent.mp4"),
            output: None,
            readiness: BatchReadiness::NeedsAttention,
            reason: Some(reason),
            included: true,
            can_edit: true,
            overridden: false,
            inspection_failed: false,
            can_toggle_included: true,
        };
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let render = |events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(730.0);
                    let _ = show(
                        ui,
                        Operation::AudioExtract,
                        std::slice::from_ref(&item),
                        crate::app::BatchAvailability {
                            can_configure: true,
                            can_add_files: true,
                            can_enqueue: false,
                        },
                        0,
                        true,
                        BatchFeedback {
                            status: None,
                            append_error: None,
                            append_error_details_open: &mut false,
                        },
                    );
                },
            );
            output.textures_delta.clear();
            output
        };
        fn status_underlined(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Text(text) => {
                    text.galley.job.text == "Needs review" && text.underline.width > 0.0
                }
                egui::Shape::Vec(shapes) => shapes.iter().any(status_underlined),
                _ => false,
            }
        }
        let underlined = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .any(|shape| status_underlined(&shape.shape))
        };
        let idle = render(vec![]);
        let status = output_text_rects(&idle, "Needs review")[0].center();
        assert!(output_text_rects(&idle, "Details").is_empty());
        assert!(output_text_rects(&idle, "1 included file needs review or inspection.").is_empty());
        assert!(output_text_rects(&idle, reason).is_empty());
        assert!(!underlined(&idle));
        let hovered = render(vec![egui::Event::PointerMoved(status)]);
        assert!(underlined(&hovered));
        assert_eq!(
            hovered.platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        for pressed in [true, false] {
            render(vec![egui::Event::PointerButton {
                pos: status,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        let opened = render(vec![]);
        assert!(!output_text_rects(&opened, reason).is_empty());
        assert!(!output_text_rects(&opened, "Close").is_empty());
    }

    #[test]
    fn mixed_batch_rows_keep_status_and_toggle_slots_aligned() {
        use super::{action_sizes, batch_action_width, batch_state_width, show_item};
        use crate::app::{BatchAction, BatchItemId};
        use crate::ui::layout;
        use std::cell::Cell;
        let mut item = BatchItemView {
            id: BatchItemId(0),
            source: Path::new("a-very-long-source-name-that-must-truncate-inside-its-own-cell.mp4"),
            output: Some(Path::new(
                "a-very-long-output-name-that-must-truncate-inside-its-own-cell.mkv",
            )),
            readiness: BatchReadiness::NeedsAttention,
            reason: Some("A detailed reason that must not occupy the status column"),
            included: true,
            can_edit: true,
            overridden: true,
            inspection_failed: false,
            can_toggle_included: true,
        };
        for width in [
            layout::MAX_CONTENT_WIDTH,
            layout::MINIMUM_CONTENT_SIZE.x - 2.0 * f32::from(layout::CONTENT_PADDING),
        ] {
            let context = egui::Context::default();
            fonts::install(&context);
            theme::install(&context);
            let render = |item: &BatchItemView<'_>| {
                let mut output = context.run_ui(Default::default(), |ui| {
                    ui.set_width(width);
                    let plan = layout::batch_row_plan(
                        width,
                        batch_action_width(ui),
                        batch_state_width(ui),
                        60.0,
                    );
                    let (_, toggle) = action_sizes(ui);
                    assert!(
                        toggle.x
                            >= crate::ui::widgets::plain_button_intrinsic_size(ui, "Include").x
                    );
                    show_item(ui, item.id, item, plan, &Cell::new(BatchAction::None));
                });
                output.textures_delta.clear();
                output
            };
            item.included = true;
            let included = render(&item);
            let status = output_text_rects(&included, "Needs review")[0];
            let edit = output_text_rects(&included, "Edit settings")[0];
            let skip = output_text_rects(&included, "Skip")[0];
            assert!(
                status.right() <= edit.left(),
                "{width}: {status:?}, {edit:?}"
            );
            assert!(output_text_rects(&included, "Details").is_empty());
            assert!(edit.right() <= skip.left());
            item.included = false;
            let skipped = render(&item);
            let include = output_text_rects(&skipped, "Include")[0];
            let edit_after = output_text_rects(&skipped, "Edit settings")[0];
            assert!(
                (skip.center() - include.center()).length() <= 1.0,
                "{width}: {skip:?}, {include:?}"
            );
            assert!((edit.center() - edit_after.center()).length() <= 1.0);
            assert_eq!(output_text_rects(&skipped, "Skipped").len(), 1);
            assert!(output_text_rects(&skipped, "Details").is_empty());
        }
    }
}

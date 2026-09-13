use std::{cell::Cell, path::Path};

use egui::{Align, Label, Layout, RichText, Sides, TextStyle, Ui};

use super::{fonts, layout, media_lifecycle, theme, widgets, workspace::Operation};

const TITLE: &str = "Batch setup";
const DEFAULTS_LABEL: &str = "Configure defaults";
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatchAction {
    SelectOperation(Operation),
    ConfigureDefaults,
    EditItem(usize),
    ToggleIncluded(usize),
    Enqueue,
    Back,
    #[default]
    None,
}

pub fn show(
    ui: &mut Ui,
    operation: Operation,
    items: &[BatchItemView<'_>],
    can_configure: bool,
    can_enqueue: bool,
    error: Option<&str>,
) -> BatchAction {
    let action = Cell::new(BatchAction::None);
    layout::operation_surface(
        ui,
        |ui| {
            show_header(ui, items, can_configure, &action);
            ui.add_space(theme::SPACING.related);
            show_operation_switcher(ui, operation, &action);
            ui.add_space(theme::SPACING.related);
            for (index, item) in items.iter().enumerate() {
                widgets::separator(ui);
                ui.add_space(theme::SPACING.compact);
                show_item(ui, index, item, &action);
                ui.add_space(theme::SPACING.compact);
            }
            widgets::separator(ui);
            if let Some(error) = error {
                ui.add_space(theme::SPACING.related);
                let _ = widgets::inline_status(ui, error, widgets::InlineStatusKind::Error);
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
    action: &Cell<BatchAction>,
) {
    let included = items.iter().filter(|item| item.included).count();
    Sides::new().height(theme::COMPACT_CONTROL_HEIGHT).show(
        ui,
        |ui| {
            ui.add(
                Label::new(RichText::new(TITLE).text_style(TextStyle::Heading)).selectable(false),
            );
            ui.add_space(theme::SPACING.compact);
            media_lifecycle::technical_faint(
                ui,
                &format!("{included} of {} included", items.len()),
            );
        },
        |ui| {
            if ui
                .add_enabled_ui(can_configure, |ui| {
                    widgets::plain_button(ui, DEFAULTS_LABEL)
                })
                .inner
                .clicked()
            {
                action.set(BatchAction::ConfigureDefaults);
            }
        },
    );
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

fn show_item(ui: &mut Ui, index: usize, item: &BatchItemView<'_>, action: &Cell<BatchAction>) {
    let file_name = item
        .source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let output_name = item
        .output
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy());
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add(
                Label::new(RichText::new(file_name).text_style(TextStyle::Body))
                    .truncate()
                    .selectable(false),
            )
            .on_hover_text(item.source.display().to_string());
            if let Some(output_name) = output_name {
                ui.add(
                    Label::new(
                        RichText::new(format!("→ {output_name}"))
                            .text_style(fonts::technical_text_style())
                            .color(theme::SECONDARY_FAINT),
                    )
                    .truncate()
                    .selectable(false),
                );
            }
            let status = item.reason.map_or_else(
                || item.readiness.label().to_owned(),
                |reason| format!("{}: {reason}", item.readiness.label()),
            );
            media_lifecycle::technical_faint(ui, &status);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let toggle_label = if item.included {
                EXCLUDE_LABEL
            } else {
                INCLUDE_LABEL
            };
            if widgets::plain_button(ui, toggle_label).clicked() {
                action.set(BatchAction::ToggleIncluded(index));
            }
            if !matches!(item.readiness, BatchReadiness::Inspecting)
                && widgets::plain_button(
                    ui,
                    if matches!(item.readiness, BatchReadiness::NeedsAttention) {
                        REVIEW_LABEL
                    } else {
                        OVERRIDE_LABEL
                    },
                )
                .clicked()
            {
                action.set(BatchAction::EditItem(index));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::BatchReadiness;

    #[test]
    fn readiness_copy_is_explicit() {
        assert_eq!(BatchReadiness::Inspecting.label(), "Inspecting");
        assert_eq!(BatchReadiness::NeedsAttention.label(), "Needs attention");
        assert_eq!(BatchReadiness::Incompatible.label(), "Incompatible");
    }
}

use std::{path::PathBuf, time::Duration};

use egui::{Align, Label, Layout, RichText, ScrollArea, Sides, TextStyle, Ui};

use crate::media::progress::MediaProgress;

use super::{fonts, layout, theme, widgets};

const PROCESSING_LABEL: &str = "Processing";
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ProgressPresentation {
    pub fraction: f32,
    pub percent: Option<String>,
    pub time: Option<String>,
    pub frame: Option<String>,
}

pub(super) fn show_progress(ui: &mut Ui, presentation: &ProgressPresentation) {
    progress_header(ui, presentation);
    ui.add_space(theme::SPACING.compact);
    let _ = widgets::progress_bar(ui, presentation.fraction);
    ui.add_space(theme::SPACING.compact);
    progress_metadata(ui, presentation);
}

pub(super) fn heading(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(RichText::new(text).text_style(TextStyle::Heading))
            .selectable(false)
            .wrap(),
    );
}

pub(super) fn secondary_copy(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(secondary_copy_text_style())
                .color(theme::SECONDARY),
        )
        .selectable(false)
        .wrap(),
    );
}

pub(super) fn secondary_copy_height(ui: &Ui, text: &str, wrap_width: f32) -> f32 {
    ui.painter()
        .layout(
            text.to_owned(),
            secondary_copy_text_style().resolve(ui.style()),
            theme::SECONDARY,
            wrap_width.max(0.0),
        )
        .size()
        .y
}

fn secondary_copy_text_style() -> TextStyle {
    TextStyle::Small
}

pub(super) fn technical_faint(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(fonts::technical_text_style())
                .color(theme::SECONDARY_FAINT),
        )
        .selectable(false)
        .wrap(),
    );
}

pub(super) fn diagnostic_viewport(ui: &mut Ui, text: &str) {
    ScrollArea::vertical()
        .max_height(layout::DIAGNOSTIC_MAX_HEIGHT)
        .min_scrolled_height(0.0)
        .auto_shrink([false, true])
        .show(ui, |ui| diagnostic_text(ui, text));
}

pub(super) fn diagnostic_height(ui: &Ui, text: &str, wrap_width: f32) -> f32 {
    ui.painter()
        .layout(
            text.to_owned(),
            fonts::technical_text_style().resolve(ui.style()),
            theme::SECONDARY_FAINT,
            wrap_width.max(0.0),
        )
        .size()
        .y
        .min(layout::DIAGNOSTIC_MAX_HEIGHT)
}

pub(super) fn file_name(ui: &mut Ui, output: &std::path::Path) {
    let name = output
        .file_name()
        .unwrap_or(output.as_os_str())
        .to_string_lossy();
    ui.add(
        Label::new(RichText::new(name).text_style(TextStyle::Body))
            .truncate()
            .selectable(false),
    );
}

pub(super) fn right_aligned_actions<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), theme::PRIMARY_CONTROL_HEIGHT),
        Layout::right_to_left(Align::Center),
        add_contents,
    )
    .inner
}

pub(super) fn progress_presentation(progress: Option<&MediaProgress>) -> ProgressPresentation {
    let fraction = progress
        .and_then(|progress| progress.fraction)
        .filter(|fraction| fraction.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let percent = progress
        .and_then(|progress| progress.fraction)
        .filter(|fraction| fraction.is_finite())
        .map(|fraction| format!("{:.0}%", fraction.clamp(0.0, 1.0) * 100.0));

    let time = progress.and_then(|progress| match (progress.processed, progress.total) {
        (Some(processed), Some(total)) => Some(format!(
            "{} / {}",
            format_duration(processed),
            format_duration(total)
        )),
        (Some(processed), None) => Some(format!("{} elapsed", format_duration(processed))),
        (None, Some(total)) => Some(format!("-- / {}", format_duration(total))),
        (None, None) => None,
    });

    let frame = progress
        .and_then(|progress| progress.frame)
        .map(|frame| format!("Frame {}", format_integer(frame)));

    ProgressPresentation {
        fraction,
        percent,
        time,
        frame,
    }
}

pub(super) fn source_to_output(input: &std::path::Path, output: &std::path::Path) -> String {
    format!(
        "{} → {}",
        file_display_name(input),
        file_display_name(output)
    )
}

pub(super) fn output_parent(output: &std::path::Path) -> PathBuf {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(super) fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

pub(super) fn format_integer(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    let first_group = digits.len() % 3;

    for (index, character) in digits.chars().enumerate() {
        if index > 0
            && (index == first_group
                || (index > first_group && (index - first_group).is_multiple_of(3)))
        {
            formatted.push(',');
        }
        formatted.push(character);
    }

    formatted
}

fn diagnostic_text(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(fonts::technical_text_style())
                .color(theme::SECONDARY_FAINT),
        )
        .selectable(true)
        .wrap(),
    );
}

fn progress_header(ui: &mut Ui, presentation: &ProgressPresentation) {
    Sides::new()
        .spacing(theme::INLINE_ACTION_GAP)
        .truncate()
        .show(
            ui,
            |ui| secondary_copy(ui, PROCESSING_LABEL),
            |ui| {
                if let Some(percent) = &presentation.percent {
                    technical_faint(ui, percent);
                }
            },
        );
}

fn progress_metadata(ui: &mut Ui, presentation: &ProgressPresentation) {
    Sides::new()
        .spacing(theme::INLINE_ACTION_GAP)
        .truncate()
        .show(
            ui,
            |ui| {
                if let Some(time) = &presentation.time {
                    technical_faint(ui, time);
                }
            },
            |ui| {
                if let Some(frame) = &presentation.frame {
                    technical_faint(ui, frame);
                }
            },
        );
}

fn file_display_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

use std::time::Duration;

use egui::{Align, Label, Layout, RichText, ScrollArea, TextStyle, Ui};

use super::{fonts, layout, theme};

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

pub(super) fn right_aligned_actions<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), theme::PRIMARY_CONTROL_HEIGHT),
        Layout::right_to_left(Align::Center),
        add_contents,
    )
    .inner
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

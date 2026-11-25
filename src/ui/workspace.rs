//! Shared source identity, plus the fixed-source egui editors retained for U02/U06.
use super::{fonts, layout, media_lifecycle, theme, widgets};
use crate::{app::Operation, media::model::MediaFile};
use egui::{Label, RichText, TextStyle, Ui};
use std::{cell::Cell, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePresentation {
    pub(crate) file_name: String,
    pub(crate) metadata: Option<String>,
}

impl WorkspacePresentation {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            file_name: source_file_name(&media.path),
            metadata: source_metadata(media),
        }
    }
}

pub fn show_fixed_editor(
    ui: &mut Ui,
    presentation: &WorkspacePresentation,
    active_operation: Operation,
    context: &str,
    enabled: bool,
    operation_switching: bool,
) -> Option<Operation> {
    let action = Cell::new(None);
    ui.add_enabled_ui(enabled, |ui| {
        show_fixed_source_header(ui, presentation, context);
        ui.add_space(theme::SPACING.related);
        ui.add_enabled_ui(operation_switching, |ui| {
            show_operation_switcher(ui, active_operation, &action);
        });
    });
    action.get()
}

fn show_fixed_source_header(ui: &mut Ui, presentation: &WorkspacePresentation, context: &str) {
    ui.spacing_mut().item_spacing.y = theme::SPACING.tight;
    ui.add(
        Label::new(
            RichText::new(context)
                .text_style(fonts::technical_text_style())
                .color(theme::SECONDARY_FAINT),
        )
        .selectable(false),
    );
    ui.add(
        Label::new(RichText::new(&presentation.file_name).text_style(TextStyle::Body))
            .truncate()
            .selectable(false),
    );
    if let Some(metadata) = presentation.metadata.as_deref() {
        ui.add(
            Label::new(
                RichText::new(metadata)
                    .text_style(fonts::technical_text_style())
                    .color(theme::SECONDARY_FAINT),
            )
            .truncate()
            .selectable(false),
        );
    }
}

fn show_operation_switcher(
    ui: &mut Ui,
    active_operation: Operation,
    action: &Cell<Option<Operation>>,
) {
    let widths = Operation::ALL
        .map(|operation| widgets::mode_button_intrinsic_size(ui, operation.label()).x);
    if layout::workspace_switcher_fits_one_line(ui.available_width(), &widths) {
        show_operation_row(ui, &Operation::ALL, active_operation, action);
    } else {
        show_operation_row(ui, &Operation::ALL[..3], active_operation, action);
        ui.add_space(theme::SPACING.compact);
        show_operation_row(ui, &Operation::ALL[3..], active_operation, action);
    }
}

fn show_operation_row(
    ui: &mut Ui,
    operations: &[Operation],
    active_operation: Operation,
    action: &Cell<Option<Operation>>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACING.related;
        for &operation in operations {
            if widgets::mode_button(ui, operation.label(), operation == active_operation).clicked()
            {
                action.set(Some(operation));
            }
        }
    });
}

fn source_file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Untitled media".to_owned())
}

fn source_metadata(media: &MediaFile) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(video) = media.video_streams.first()
        && let (Some(width), Some(height)) = (video.width, video.height)
        && width > 0
        && height > 0
    {
        parts.push(format!("{width}×{height}"));
    }
    if let Some(duration) = media.duration {
        parts.push(media_lifecycle::format_duration(duration));
    }
    if parts.is_empty() && !media.streams.is_empty() {
        parts.push(format!(
            "{} {}",
            media.streams.len(),
            if media.streams.len() == 1 {
                "stream"
            } else {
                "streams"
            }
        ));
    }

    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::model::{AudioStream, MediaStream, StreamKind, VideoStream};
    use std::{path::PathBuf, time::Duration};
    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("recording.mkv"),
            duration: Some(Duration::from_secs(151)),
            streams: vec![
                MediaStream {
                    stream_index: 0,
                    kind: StreamKind::Video,
                    codec: Some("h264".to_owned()),
                },
                MediaStream {
                    stream_index: 1,
                    kind: StreamKind::Audio,
                    codec: Some("aac".to_owned()),
                },
            ],
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: vec![AudioStream {
                stream_index: 1,
                codec: Some("aac".to_owned()),
                channels: Some(2),
                channel_layout: Some("stereo".to_owned()),
                title: None,
                language: None,
            }],
        }
    }

    #[test]
    fn shared_metadata_uses_source_dimensions_and_duration() {
        assert_eq!(
            source_metadata(&media()).as_deref(),
            Some("1920×1080 · 02:31")
        );
    }
}

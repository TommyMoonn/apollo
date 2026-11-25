use crate::remux::draft::RemuxDraft;

use std::{cell::Cell, ffi::OsString};

use egui::Ui;

use crate::{
    media::{
        container::MediaContainer,
        model::{MediaFile, StreamKind},
        output::MediaOutputError,
    },
    remux::validation::RemuxValidationError,
};

use super::{
    OperationSurfaceMode, layout, media_error, show_queued_submission_actions, theme, widgets,
};

const OUTPUT_LABEL: &str = "Output";
const CONTAINER_LABEL: &str = "Container";
const BROWSE_LABEL: &str = "Browse";
const CONTAINER_OPTIONS: [&str; 2] = ["MP4", "MKV"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemuxPresentation {
    mp4_compatible: bool,
    matroska_compatible: bool,
    streams: Vec<RemuxStreamPresentation>,
}

impl RemuxPresentation {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            mp4_compatible: MediaContainer::Mp4.is_inventory_stream_copy_compatible(&media.streams),
            matroska_compatible: MediaContainer::Matroska
                .is_inventory_stream_copy_compatible(&media.streams),
            streams: media
                .streams
                .iter()
                .map(|stream| RemuxStreamPresentation {
                    stream_index: stream.stream_index,
                    description: stream_description(
                        stream.stream_index,
                        stream.kind,
                        stream.codec.as_deref(),
                    ),
                })
                .collect(),
        }
    }

    fn container_compatible(&self, container: MediaContainer) -> bool {
        match container {
            MediaContainer::Mp4 => self.mp4_compatible,
            MediaContainer::Matroska => self.matroska_compatible,
        }
    }

    fn stream_description(&self, stream_index: u32) -> &str {
        self.streams
            .iter()
            .find(|stream| stream.stream_index == stream_index)
            .map(|stream| stream.description.as_str())
            .unwrap_or("Unknown source stream")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemuxStreamPresentation {
    stream_index: u32,
    description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RemuxSelectionAction {
    #[default]
    None,
    BrowseOutput,
    QueueRequested,
    RemuxRequested,
    SaveQueueChanges,
    CancelQueueEdit,
}

pub fn show<'a>(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &RemuxPresentation,
    state: &mut RemuxDraft,
    mode: impl Into<OperationSurfaceMode<'a>>,
) -> RemuxSelectionAction {
    let mode = mode.into();
    let shared_defaults = matches!(mode, OperationSurfaceMode::BatchDefaults { .. });
    let action = Cell::new(RemuxSelectionAction::None);
    let validation = state.validation(media);
    let feedback = validation
        .as_ref()
        .err()
        .and_then(|error| remux_ready_feedback(error, presentation));
    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            let _ = widgets::section_label(
                ui,
                if shared_defaults {
                    CONTAINER_LABEL
                } else {
                    OUTPUT_LABEL
                },
            );
            ui.add_space(theme::SPACING.compact);
            if shared_defaults {
                show_container_selector(ui, state);
            } else {
                let mut output_stem = state.output_filename_stem().to_string_lossy().into_owned();
                let selector_width = widgets::select_menu_intrinsic_width(ui, &CONTAINER_OPTIONS);
                let row = widgets::operation_output_row(
                    ui,
                    &mut output_stem,
                    state.container().extension(),
                    BROWSE_LABEL,
                    selector_width,
                    |ui| show_container_selector(ui, state),
                );
                if row.filename.changed() {
                    state.set_output_filename_stem(OsString::from(output_stem));
                }
                if row.browse.clicked() {
                    action.set(RemuxSelectionAction::BrowseOutput);
                }
            }

            if let Some(feedback) = feedback.as_deref() {
                ui.add_space(theme::SPACING.tight);
                let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
            }
            if let Some(error) = mode.inline_action_error() {
                ui.add_space(theme::SPACING.related);
                media_error::show_operation_action(ui, error);
            }
        },
        |ui| match mode {
            OperationSurfaceMode::Normal {
                process_now_allowed,
                queue_confirmation,
                ..
            } => {
                match show_queued_submission_actions(
                    ui,
                    super::PROCESS_NOW_LABEL,
                    process_now_allowed,
                    validation.is_ok(),
                    queue_confirmation,
                ) {
                    Some(crate::app::queue::QueueSubmission::ProcessNow) => {
                        action.set(RemuxSelectionAction::RemuxRequested)
                    }
                    Some(crate::app::queue::QueueSubmission::AddToQueue) => {
                        action.set(RemuxSelectionAction::QueueRequested)
                    }
                    None => {}
                }
            }
            edit_mode => {
                let (save_label, cancel_label) = edit_mode
                    .edit_labels()
                    .expect("editor mode has edit labels");
                let save_allowed = match edit_mode {
                    OperationSurfaceMode::BatchDefaults { save_allowed } => save_allowed,
                    _ => validation.is_ok(),
                };
                if widgets::primary_button_enabled(ui, save_label, save_allowed).clicked() {
                    action.set(RemuxSelectionAction::SaveQueueChanges);
                }
                if widgets::secondary_button(ui, cancel_label).clicked() {
                    action.set(RemuxSelectionAction::CancelQueueEdit);
                }
            }
        },
    );

    action.get()
}

fn show_container_selector(ui: &mut Ui, state: &mut RemuxDraft) {
    let mut requested = None;
    widgets::select_menu(
        ui,
        container_label(state.container()),
        &CONTAINER_OPTIONS,
        |ui| {
            if widgets::select_menu_option(ui, "MP4", state.container() == MediaContainer::Mp4)
                .clicked()
            {
                requested = Some(MediaContainer::Mp4);
                ui.close();
            }
            if widgets::select_menu_option(ui, "MKV", state.container() == MediaContainer::Matroska)
                .clicked()
            {
                requested = Some(MediaContainer::Matroska);
                ui.close();
            }
        },
    );
    if let Some(container) = requested {
        state.set_container(container);
    }
}

pub(super) fn remux_ready_feedback(
    error: &RemuxValidationError,
    presentation: &RemuxPresentation,
) -> Option<String> {
    match error {
        RemuxValidationError::NoStreams => {
            Some("This source has no media streams to remux.".to_owned())
        }
        RemuxValidationError::IncompatibleStream {
            container,
            stream_index,
            ..
        } => {
            let selected = container_label(*container);
            let other = match container {
                MediaContainer::Mp4 => MediaContainer::Matroska,
                MediaContainer::Matroska => MediaContainer::Mp4,
            };
            let stream = presentation.stream_description(*stream_index);
            if presentation.container_compatible(other) {
                Some(format!(
                    "{stream} cannot be copied into {selected}. Use {} instead.",
                    container_label(other)
                ))
            } else {
                Some(format!(
                    "{stream} cannot be copied into {selected}. Neither MP4 nor MKV can copy every source stream."
                ))
            }
        }
        RemuxValidationError::OutputExtensionMismatch { actual: None, .. }
        | RemuxValidationError::Output(MediaOutputError::InvalidOutputPath(_)) => {
            Some("Enter a usable output filename.".to_owned())
        }
        RemuxValidationError::OutputExtensionMismatch { .. } => {
            Some("Output filename must match the selected container.".to_owned())
        }
        RemuxValidationError::Output(_) => None,
    }
}

fn stream_description(stream_index: u32, kind: StreamKind, codec: Option<&str>) -> String {
    let codec = codec
        .filter(|codec| !codec.trim().is_empty())
        .map(|codec| codec.to_ascii_uppercase())
        .unwrap_or_else(|| "codec unknown".to_owned());
    format!(
        "Stream {stream_index} ({}, {codec})",
        stream_kind_label(kind)
    )
}

fn stream_kind_label(kind: StreamKind) -> &'static str {
    match kind {
        StreamKind::Video => "video",
        StreamKind::Audio => "audio",
        StreamKind::Subtitle => "subtitle",
        StreamKind::Data => "data",
        StreamKind::Attachment => "attachment",
        StreamKind::Unknown => "unknown",
    }
}

pub(super) fn container_label(container: MediaContainer) -> &'static str {
    match container {
        MediaContainer::Mp4 => "MP4",
        MediaContainer::Matroska => "MKV",
    }
}

#[cfg(test)]
mod tests {

    use crate::{
        media::{
            container::MediaContainer,
            model::{MediaStream, StreamKind},
        },
        remux::validation::RemuxValidationError,
    };

    use super::{RemuxPresentation, remux_ready_feedback};

    use crate::remux::draft::tests::media_with_streams;

    #[test]
    fn incompatible_mp4_feedback_is_visible_before_execution() {
        let media = media_with_streams(vec![
            MediaStream {
                stream_index: 0,
                kind: StreamKind::Video,
                codec: Some("h264".to_owned()),
            },
            MediaStream {
                stream_index: 4,
                kind: StreamKind::Audio,
                codec: Some("wmav2".to_owned()),
            },
        ]);
        let presentation = RemuxPresentation::new(&media);
        assert_eq!(
            remux_ready_feedback(
                &RemuxValidationError::IncompatibleStream {
                    container: MediaContainer::Mp4,
                    stream_index: 4,
                    kind: StreamKind::Audio,
                    codec: Some("wmav2".to_owned()),
                },
                &presentation,
            ),
            Some("Stream 4 (audio, WMAV2) cannot be copied into MP4. Use MKV instead.".to_owned())
        );
    }

    #[test]
    fn inventory_incompatible_with_both_containers_does_not_recommend_mkv() {
        let media = media_with_streams(vec![MediaStream {
            stream_index: 15,
            kind: StreamKind::Data,
            codec: Some("bin_data".to_owned()),
        }]);
        let presentation = RemuxPresentation::new(&media);
        let feedback = remux_ready_feedback(
            &RemuxValidationError::IncompatibleStream {
                container: MediaContainer::Mp4,
                stream_index: 15,
                kind: StreamKind::Data,
                codec: Some("bin_data".to_owned()),
            },
            &presentation,
        )
        .unwrap();

        assert_eq!(
            feedback,
            "Stream 15 (data, BIN_DATA) cannot be copied into MP4. Neither MP4 nor MKV can copy every source stream."
        );
        assert!(!feedback.contains("Use MKV instead"));
    }
}

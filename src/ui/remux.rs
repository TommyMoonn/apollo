use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use egui::Ui;

use crate::{
    media::{
        container::MediaContainer,
        model::{MediaFile, StreamKind},
        output::MediaOutputError,
    },
    remux::{
        model::RemuxRequest,
        validation::{RemuxValidationError, validate_remux_request},
    },
};

use super::{
    ADD_TO_QUEUE_LABEL, OperationSurfaceMode, layout, output_name::OutputNameState,
    show_queue_confirmation, theme, widgets,
};

const OUTPUT_LABEL: &str = "Output";
const CONTAINER_LABEL: &str = "Container";
const BROWSE_LABEL: &str = "Browse";
const REMUX_LABEL: &str = "Remux";
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemuxSelectionState {
    container: MediaContainer,
    output_name: OutputNameState,
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

impl RemuxSelectionState {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            container: MediaContainer::Matroska,
            output_name: OutputNameState::new(&media.path, default_output_stem(&media.path)),
        }
    }

    pub fn from_request(media: &MediaFile, request: &RemuxRequest) -> Self {
        let mut state = Self {
            container: request.container,
            output_name: OutputNameState::new(&media.path, default_output_stem(&media.path)),
        };
        state.output_name.set_custom_path(request.output.clone());
        state
    }

    pub fn container(&self) -> MediaContainer {
        self.container
    }

    pub fn set_container(&mut self, container: MediaContainer) {
        self.container = container;
    }

    pub fn output_path(&self) -> PathBuf {
        self.output_name.path(self.container.extension())
    }

    pub fn output_filename_stem(&self) -> &OsStr {
        self.output_name.stem()
    }

    pub fn output_is_customized(&self) -> bool {
        self.output_name.is_customized()
    }

    pub fn set_output_filename_stem(&mut self, stem: OsString) {
        self.output_name.set_stem(stem);
    }

    pub fn set_custom_output_path(&mut self, output: PathBuf) {
        self.output_name.set_custom_path(output);
    }

    pub fn can_remux(&self, media: &MediaFile) -> bool {
        self.validation(media).is_ok()
    }

    pub fn request(&self, media: &MediaFile) -> RemuxRequest {
        RemuxRequest {
            input: media.path.clone(),
            output: self.output_path(),
            container: self.container,
        }
    }

    fn validation(&self, media: &MediaFile) -> Result<(), RemuxValidationError> {
        validate_remux_request(&self.request(media), media)
    }
}

pub fn show(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &RemuxPresentation,
    state: &mut RemuxSelectionState,
    mode: impl Into<OperationSurfaceMode>,
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
        },
        |ui| match mode {
            OperationSurfaceMode::Immediate {
                start_allowed,
                queue_confirmation,
            } => {
                if widgets::primary_button_enabled(
                    ui,
                    REMUX_LABEL,
                    validation.is_ok() && start_allowed,
                )
                .clicked()
                {
                    action.set(RemuxSelectionAction::RemuxRequested);
                }
                if ui
                    .add_enabled_ui(validation.is_ok(), |ui| {
                        widgets::secondary_button(ui, ADD_TO_QUEUE_LABEL)
                    })
                    .inner
                    .clicked()
                {
                    action.set(RemuxSelectionAction::QueueRequested);
                }
                show_queue_confirmation(ui, queue_confirmation);
            }
            edit_mode => {
                let (save_label, cancel_label) = edit_mode
                    .edit_labels()
                    .expect("non-immediate operation mode has edit labels");
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

fn show_container_selector(ui: &mut Ui, state: &mut RemuxSelectionState) {
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

fn remux_ready_feedback(
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

fn default_output_stem(input: &Path) -> OsString {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));
    let mut output = OsString::from(stem);
    output.push("-remuxed");
    output
}

pub(super) fn container_label(container: MediaContainer) -> &'static str {
    match container {
        MediaContainer::Mp4 => "MP4",
        MediaContainer::Matroska => "MKV",
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use crate::{
        media::{
            container::MediaContainer,
            model::{MediaFile, MediaStream, StreamKind},
        },
        remux::validation::RemuxValidationError,
    };

    use super::{RemuxPresentation, RemuxSelectionState, remux_ready_feedback};

    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("captures/recording.mkv"),
            duration: None,
            streams: vec![MediaStream {
                stream_index: 0,
                kind: StreamKind::Video,
                codec: Some("h264".to_owned()),
            }],
            video_streams: Vec::new(),
            audio_streams: Vec::new(),
        }
    }

    #[test]
    fn default_state_uses_matroska_and_remuxed_output_name() {
        let media = media();
        let state = RemuxSelectionState::new(&media);

        assert_eq!(state.container(), MediaContainer::Matroska);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-remuxed.mkv")
        );
        assert!(!state.output_is_customized());
        assert!(state.can_remux(&media));
    }

    #[test]
    fn container_changes_only_the_authoritative_extension() {
        let media = media();
        let mut state = RemuxSelectionState::new(&media);

        state.set_container(MediaContainer::Mp4);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-remuxed.mp4")
        );
        state.set_container(MediaContainer::Matroska);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-remuxed.mkv")
        );
    }

    #[test]
    fn custom_output_stem_survives_container_changes() {
        let media = media();
        let mut state = RemuxSelectionState::new(&media);
        state.set_output_filename_stem(OsString::from("delivery copy"));

        state.set_container(MediaContainer::Mp4);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/delivery copy.mp4")
        );
        state.set_container(MediaContainer::Matroska);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/delivery copy.mkv")
        );
        assert!(state.output_is_customized());
    }

    #[test]
    fn custom_browse_path_keeps_directory_and_selected_container_extension() {
        let media = media();
        let mut state = RemuxSelectionState::new(&media);
        state.set_custom_output_path(PathBuf::from("exports/final.mp4"));

        assert_eq!(state.output_path(), PathBuf::from("exports/final.mkv"));
        state.set_container(MediaContainer::Mp4);
        assert_eq!(state.output_path(), PathBuf::from("exports/final.mp4"));
    }

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

    #[test]
    fn validation_controls_primary_enablement() {
        let mut media = media();
        let mut state = RemuxSelectionState::new(&media);
        assert!(state.can_remux(&media));

        state.set_output_filename_stem(OsString::new());
        assert!(!state.can_remux(&media));

        state = RemuxSelectionState::new(&media);
        media.streams.clear();
        assert!(!state.can_remux(&media));
    }

    fn media_with_streams(streams: Vec<MediaStream>) -> MediaFile {
        let mut media = media();
        media.streams = streams;
        media
    }
}

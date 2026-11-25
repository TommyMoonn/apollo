use crate::audio_extract::draft::{AudioExtractDraft, AudioExtractReadyError};

use std::{cell::Cell, ffi::OsString};

use egui::{Label, RichText, Ui};

use crate::{
    audio_extract::{model::AudioExtractFormat, validation::AudioExtractValidationError},
    media::{model::MediaFile, output::MediaOutputError},
};

use super::{
    ADD_TO_QUEUE_LABEL, OperationSurfaceMode, fonts, layout, media_error, show_queue_confirmation,
    theme, widgets,
};

const AUDIO_TRACK_LABEL: &str = "Audio track";
const OUTPUT_LABEL: &str = "Output";
const FORMAT_LABEL: &str = "Output format";
const BROWSE_LABEL: &str = "Browse";
const EXTRACT_LABEL: &str = "Extract";
const FORMAT_OPTIONS: [&str; 3] = ["Original (copy)", "AAC", "FLAC"];
const NO_AUDIO_FEEDBACK: &str = "This source has no audio tracks to extract.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioExtractPresentation {
    tracks: Vec<AudioTrackPresentation>,
}

impl AudioExtractPresentation {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            tracks: media
                .audio_streams
                .iter()
                .map(|stream| AudioTrackPresentation {
                    stream_index: stream.stream_index,
                    title: stream
                        .title
                        .as_deref()
                        .map(str::trim)
                        .filter(|title| !title.is_empty())
                        .unwrap_or("Untitled audio")
                        .to_owned(),
                    metadata: track_metadata(stream),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioTrackPresentation {
    stream_index: u32,
    title: String,
    metadata: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioExtractSelectionAction {
    #[default]
    None,
    BrowseOutput,
    QueueRequested,
    ExtractRequested,
    SaveQueueChanges,
    CancelQueueEdit,
}

pub fn show<'a>(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &AudioExtractPresentation,
    state: &mut AudioExtractDraft,
    mode: impl Into<OperationSurfaceMode<'a>>,
) -> AudioExtractSelectionAction {
    let mode = mode.into();
    let shared_defaults = matches!(mode, OperationSurfaceMode::BatchDefaults { .. });
    let action = Cell::new(AudioExtractSelectionAction::None);
    let validation = state.validation(media);
    let feedback = validation.as_ref().err().and_then(ready_feedback);

    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            if !shared_defaults {
                let _ = widgets::section_label(ui, AUDIO_TRACK_LABEL);
                ui.add_space(theme::SPACING.compact);
                show_audio_tracks(ui, media, presentation, state);
                ui.add_space(theme::SPACING.section);
            }
            let _ = widgets::section_label(
                ui,
                if shared_defaults {
                    FORMAT_LABEL
                } else {
                    OUTPUT_LABEL
                },
            );
            ui.add_space(theme::SPACING.compact);
            if shared_defaults {
                show_format_selector(ui, state);
            } else {
                let mut output_stem = state.output_filename_stem().to_string_lossy().into_owned();
                let selector_width = widgets::select_menu_intrinsic_width(ui, &FORMAT_OPTIONS);
                let row = widgets::operation_output_row(
                    ui,
                    &mut output_stem,
                    state.format().extension(),
                    BROWSE_LABEL,
                    selector_width,
                    |ui| show_format_selector(ui, state),
                );
                if row.filename.changed() {
                    state.set_output_filename_stem(OsString::from(output_stem));
                }
                if row.browse.clicked() {
                    action.set(AudioExtractSelectionAction::BrowseOutput);
                }
            }
            ui.add_space(theme::SPACING.tight);
            ui.add(
                Label::new(
                    RichText::new(format_detail(state.format()))
                        .text_style(fonts::technical_text_style())
                        .color(theme::SECONDARY_FAINT),
                )
                .selectable(false),
            );

            if let Some(feedback) = feedback {
                ui.add_space(theme::SPACING.tight);
                let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
            }
            if let Some(error) = mode.inline_action_error() {
                ui.add_space(theme::SPACING.related);
                media_error::show_operation_action(ui, error);
            }
        },
        |ui| match mode {
            OperationSurfaceMode::Immediate {
                start_allowed,
                queue_confirmation,
                ..
            } => {
                if widgets::primary_button_enabled(
                    ui,
                    EXTRACT_LABEL,
                    validation.is_ok() && start_allowed,
                )
                .clicked()
                {
                    action.set(AudioExtractSelectionAction::ExtractRequested);
                }
                if ui
                    .add_enabled_ui(validation.is_ok(), |ui| {
                        widgets::secondary_button(ui, ADD_TO_QUEUE_LABEL)
                    })
                    .inner
                    .clicked()
                {
                    action.set(AudioExtractSelectionAction::QueueRequested);
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
                    action.set(AudioExtractSelectionAction::SaveQueueChanges);
                }
                if widgets::secondary_button(ui, cancel_label).clicked() {
                    action.set(AudioExtractSelectionAction::CancelQueueEdit);
                }
            }
        },
    );

    action.get()
}

fn show_audio_tracks(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &AudioExtractPresentation,
    state: &mut AudioExtractDraft,
) {
    widgets::separator(ui);
    if presentation.tracks.is_empty() {
        widgets::separator(ui);
        return;
    }

    for track in &presentation.tracks {
        let response = widgets::selectable_row(
            ui,
            &track.title,
            Some(&track.metadata),
            state.selected_stream_index() == Some(track.stream_index),
            widgets::SelectionIndicator::Radio,
            theme::SELECTABLE_ROW_MIN_HEIGHT,
        );
        if response.clicked() {
            let _ = state.select_stream(media, track.stream_index);
        }
    }
}

fn track_metadata(stream: &crate::media::model::AudioStream) -> String {
    let mut parts = vec![format!("stream {}", stream.stream_index)];
    if let Some(channel) = stream
        .channel_layout
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(channel.to_owned());
    } else if let Some(channels) = stream.channels {
        parts.push(format!("{channels} ch"));
    }
    if let Some(language) = stream
        .language
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(language.to_owned());
    }
    if let Some(codec) = stream
        .codec
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(codec.to_ascii_uppercase());
    }
    parts.join(" · ")
}

fn show_format_selector(ui: &mut Ui, state: &mut AudioExtractDraft) {
    let mut requested = None;
    widgets::select_menu(ui, format_label(state.format()), &FORMAT_OPTIONS, |ui| {
        for format in [
            AudioExtractFormat::Original,
            AudioExtractFormat::Aac,
            AudioExtractFormat::Flac,
        ] {
            if widgets::select_menu_option(ui, format_label(format), state.format() == format)
                .clicked()
            {
                requested = Some(format);
                ui.close();
            }
        }
    });
    if let Some(format) = requested {
        state.set_format(format);
    }
}

fn ready_feedback(error: &AudioExtractReadyError) -> Option<&'static str> {
    match error {
        AudioExtractReadyError::NoAudioStreams => Some(NO_AUDIO_FEEDBACK),
        AudioExtractReadyError::Request(AudioExtractValidationError::OutputExtensionMismatch {
            actual: None,
            ..
        })
        | AudioExtractReadyError::Request(AudioExtractValidationError::Output(
            MediaOutputError::InvalidOutputPath(_),
        )) => Some("Enter a usable output filename."),
        AudioExtractReadyError::Request(AudioExtractValidationError::OutputExtensionMismatch {
            ..
        }) => Some("Output filename must match the selected format."),
        AudioExtractReadyError::Request(
            AudioExtractValidationError::UnknownStreamIndex { .. }
            | AudioExtractValidationError::StreamIsNotAudio { .. },
        ) => Some("Choose an available audio track."),
        AudioExtractReadyError::Request(AudioExtractValidationError::Output(_)) => None,
    }
}

pub(super) fn format_label(format: AudioExtractFormat) -> &'static str {
    match format {
        AudioExtractFormat::Original => "Original (copy)",
        AudioExtractFormat::Aac => "AAC",
        AudioExtractFormat::Flac => "FLAC",
    }
}

fn format_detail(format: AudioExtractFormat) -> &'static str {
    match format {
        AudioExtractFormat::Original => "no re-encode · MKA",
        AudioExtractFormat::Aac => "256 kbps · M4A",
        AudioExtractFormat::Flac => "lossless · FLAC",
    }
}

#[cfg(test)]
mod tests {

    use crate::media::model::StreamKind;

    use super::{AudioExtractDraft, NO_AUDIO_FEEDBACK, ready_feedback};

    use crate::audio_extract::draft::tests::media;

    #[test]
    fn no_audio_source_is_non_runnable_with_clear_feedback() {
        let mut media = media();
        media.audio_streams.clear();
        media
            .streams
            .retain(|stream| stream.kind != StreamKind::Audio);
        let state = AudioExtractDraft::new(&media);
        assert_eq!(state.selected_stream_index(), None);
        assert!(!state.can_extract(&media));
        assert_eq!(
            ready_feedback(&state.validation(&media).unwrap_err()),
            Some(NO_AUDIO_FEEDBACK)
        );
    }
}

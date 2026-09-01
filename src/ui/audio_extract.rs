use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use egui::{Label, RichText, Ui};

use crate::{
    audio_extract::{
        model::{AudioExtractFormat, AudioExtractRequest},
        validation::{AudioExtractValidationError, validate_audio_extract_request},
    },
    media::{model::MediaFile, output::MediaOutputError},
};

use super::{fonts, layout, output_name::OutputNameState, theme, widgets};

const AUDIO_TRACK_LABEL: &str = "Audio track";
const OUTPUT_LABEL: &str = "Output";
const BROWSE_LABEL: &str = "Browse";
const EXTRACT_LABEL: &str = "Extract";
const FORMAT_OPTIONS: [&str; 3] = ["Original (copy)", "AAC", "FLAC"];
const NO_AUDIO_FEEDBACK: &str = "This source has no audio tracks to extract.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioExtractSelectionState {
    selected_stream_index: Option<u32>,
    format: AudioExtractFormat,
    output_name: OutputNameState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioExtractSelectionAction {
    #[default]
    None,
    BrowseOutput,
    ExtractRequested,
}

impl AudioExtractSelectionState {
    pub fn new(media: &MediaFile) -> Self {
        let selected_stream_index = media
            .audio_streams
            .first()
            .map(|stream| stream.stream_index);
        Self {
            selected_stream_index,
            format: AudioExtractFormat::Original,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(
                    &media.path,
                    media.audio_streams.len(),
                    selected_stream_index,
                ),
            ),
        }
    }

    pub fn selected_stream_index(&self) -> Option<u32> {
        self.selected_stream_index
    }

    pub fn select_stream(&mut self, media: &MediaFile, stream_index: u32) -> bool {
        if !media
            .audio_streams
            .iter()
            .any(|stream| stream.stream_index == stream_index)
        {
            return false;
        }
        self.selected_stream_index = Some(stream_index);
        self.output_name.set_generated_stem(default_output_stem(
            &media.path,
            media.audio_streams.len(),
            self.selected_stream_index,
        ));
        true
    }

    pub fn format(&self) -> AudioExtractFormat {
        self.format
    }

    pub fn set_format(&mut self, format: AudioExtractFormat) {
        self.format = format;
    }

    pub fn output_path(&self) -> PathBuf {
        self.output_name.path(self.format.extension())
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

    pub fn request(&self, media: &MediaFile) -> Option<AudioExtractRequest> {
        Some(AudioExtractRequest {
            input: media.path.clone(),
            output: self.output_path(),
            audio_stream_index: self.selected_stream_index?,
            format: self.format,
        })
    }

    pub fn can_extract(&self, media: &MediaFile) -> bool {
        self.validation(media).is_ok()
    }

    fn validation(&self, media: &MediaFile) -> Result<(), AudioExtractReadyError> {
        let request = self
            .request(media)
            .ok_or(AudioExtractReadyError::NoAudioStreams)?;
        validate_audio_extract_request(&request, media).map_err(AudioExtractReadyError::Request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AudioExtractReadyError {
    NoAudioStreams,
    Request(AudioExtractValidationError),
}

pub fn show(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut AudioExtractSelectionState,
) -> AudioExtractSelectionAction {
    let action = Cell::new(AudioExtractSelectionAction::None);
    let validation = state.validation(media);
    let feedback = validation.as_ref().err().and_then(ready_feedback);

    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            let _ = widgets::section_label(ui, AUDIO_TRACK_LABEL);
            ui.add_space(theme::SPACING.compact);
            show_audio_tracks(ui, media, state);

            ui.add_space(theme::SPACING.section);
            let _ = widgets::section_label(ui, OUTPUT_LABEL);
            ui.add_space(theme::SPACING.compact);
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
        },
        |ui| {
            if widgets::primary_button_enabled(ui, EXTRACT_LABEL, validation.is_ok()).clicked() {
                action.set(AudioExtractSelectionAction::ExtractRequested);
            }
        },
    );

    action.get()
}

fn show_audio_tracks(ui: &mut Ui, media: &MediaFile, state: &mut AudioExtractSelectionState) {
    widgets::separator(ui);
    if media.audio_streams.is_empty() {
        widgets::separator(ui);
        return;
    }

    for stream in &media.audio_streams {
        let title = stream
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("Untitled audio");
        let metadata = track_metadata(stream);
        let response = widgets::selectable_row(
            ui,
            title,
            Some(&metadata),
            state.selected_stream_index() == Some(stream.stream_index),
            widgets::SelectionIndicator::Radio,
            theme::SELECTABLE_ROW_MIN_HEIGHT,
        );
        if response.clicked() {
            let _ = state.select_stream(media, stream.stream_index);
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

fn show_format_selector(ui: &mut Ui, state: &mut AudioExtractSelectionState) {
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

fn default_output_stem(
    input: &Path,
    audio_stream_count: usize,
    selected_stream_index: Option<u32>,
) -> OsString {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));
    let mut output = OsString::from(stem);
    output.push("-audio");
    if audio_stream_count > 1
        && let Some(stream_index) = selected_stream_index
    {
        output.push(format!("-{stream_index}"));
    }
    output
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
    use std::{ffi::OsString, path::PathBuf};

    use crate::{
        audio_extract::model::AudioExtractFormat,
        media::model::{AudioStream, MediaFile, MediaStream, StreamKind},
    };

    use super::{
        AudioExtractSelectionState, NO_AUDIO_FEEDBACK, format_detail, format_label, ready_feedback,
    };

    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("captures/recording.mkv"),
            duration: None,
            streams: vec![
                MediaStream {
                    stream_index: 0,
                    kind: StreamKind::Video,
                    codec: Some("h264".into()),
                },
                MediaStream {
                    stream_index: 3,
                    kind: StreamKind::Audio,
                    codec: Some("aac".into()),
                },
                MediaStream {
                    stream_index: 7,
                    kind: StreamKind::Audio,
                    codec: Some("opus".into()),
                },
            ],
            video_streams: Vec::new(),
            audio_streams: vec![
                AudioStream {
                    stream_index: 3,
                    codec: Some("aac".into()),
                    channels: Some(2),
                    channel_layout: Some("stereo".into()),
                    title: Some("Program".into()),
                    language: None,
                },
                AudioStream {
                    stream_index: 7,
                    codec: Some("opus".into()),
                    channels: Some(2),
                    channel_layout: Some("stereo".into()),
                    title: Some("Commentary".into()),
                    language: Some("eng".into()),
                },
            ],
        }
    }

    #[test]
    fn multi_track_source_defaults_to_first_absolute_index_and_indexed_audio_suffix() {
        let media = media();
        let mut state = AudioExtractSelectionState::new(&media);
        assert_eq!(state.selected_stream_index(), Some(3));
        assert_eq!(state.format(), AudioExtractFormat::Original);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-audio-3.mka")
        );
        assert!(state.can_extract(&media));
        assert_eq!(state.request(&media).unwrap().audio_stream_index, 3);

        state.set_format(AudioExtractFormat::Aac);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-audio-3.m4a")
        );
    }

    #[test]
    fn single_track_source_keeps_the_simple_audio_suffix() {
        let mut media = media();
        media.audio_streams.truncate(1);
        media
            .streams
            .retain(|stream| stream.kind != StreamKind::Audio || stream.stream_index == 3);

        let state = AudioExtractSelectionState::new(&media);

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-audio.mka")
        );
    }

    #[test]
    fn selection_uses_only_probed_absolute_audio_indexes() {
        let media = media();
        let mut state = AudioExtractSelectionState::new(&media);
        assert!(state.select_stream(&media, 7));
        assert_eq!(state.request(&media).unwrap().audio_stream_index, 7);
        assert!(!state.select_stream(&media, 0));
        assert!(!state.select_stream(&media, 99));
        assert_eq!(state.selected_stream_index(), Some(7));
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-audio-7.mka")
        );
    }

    #[test]
    fn customized_output_stem_survives_later_track_and_format_changes() {
        let media = media();
        let mut state = AudioExtractSelectionState::new(&media);
        state.set_output_filename_stem(OsString::from("chosen commentary"));

        assert!(state.select_stream(&media, 7));
        state.set_format(AudioExtractFormat::Flac);

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/chosen commentary.flac")
        );
        assert!(state.output_is_customized());
    }

    #[test]
    fn format_labels_and_details_match_the_command_policy() {
        assert_eq!(
            format_label(AudioExtractFormat::Original),
            "Original (copy)"
        );
        assert_eq!(
            format_detail(AudioExtractFormat::Original),
            "no re-encode · MKA"
        );
        assert_eq!(format_label(AudioExtractFormat::Aac), "AAC");
        assert_eq!(format_detail(AudioExtractFormat::Aac), "256 kbps · M4A");
        assert_eq!(format_label(AudioExtractFormat::Flac), "FLAC");
        assert_eq!(format_detail(AudioExtractFormat::Flac), "lossless · FLAC");
    }

    #[test]
    fn format_changes_authoritative_extension_and_preserves_custom_stem() {
        let media = media();
        let mut state = AudioExtractSelectionState::new(&media);
        state.set_output_filename_stem(OsString::from("dialogue final"));
        for (format, expected) in [
            (AudioExtractFormat::Aac, "captures/dialogue final.m4a"),
            (AudioExtractFormat::Flac, "captures/dialogue final.flac"),
            (AudioExtractFormat::Original, "captures/dialogue final.mka"),
        ] {
            state.set_format(format);
            assert_eq!(state.output_path(), PathBuf::from(expected));
        }
        assert!(state.output_is_customized());
    }

    #[test]
    fn browse_path_preserves_directory_while_format_owns_extension() {
        let media = media();
        let mut state = AudioExtractSelectionState::new(&media);
        state.set_custom_output_path(PathBuf::from("exports/voice.flac"));
        assert_eq!(state.output_path(), PathBuf::from("exports/voice.mka"));
        state.set_format(AudioExtractFormat::Flac);
        assert_eq!(state.output_path(), PathBuf::from("exports/voice.flac"));
    }

    #[test]
    fn no_audio_source_is_non_runnable_with_clear_feedback() {
        let mut media = media();
        media.audio_streams.clear();
        media
            .streams
            .retain(|stream| stream.kind != StreamKind::Audio);
        let state = AudioExtractSelectionState::new(&media);
        assert_eq!(state.selected_stream_index(), None);
        assert!(!state.can_extract(&media));
        assert_eq!(
            ready_feedback(&state.validation(&media).unwrap_err()),
            Some(NO_AUDIO_FEEDBACK)
        );
    }
}

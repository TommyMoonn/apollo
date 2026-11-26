//! Editable operation configuration and typed request construction.

use super::{
    model::{AudioExtractFormat, AudioExtractRequest},
    validation::{AudioExtractValidationError, validate_audio_extract_request},
};
use crate::media::{model::MediaFile, output_name::OutputNameState};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioExtractDraft {
    selected_stream_index: Option<u32>,
    format: AudioExtractFormat,
    output_name: OutputNameState,
}

impl AudioExtractDraft {
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

    pub fn from_request(media: &MediaFile, request: &AudioExtractRequest) -> Self {
        let mut state = Self {
            selected_stream_index: Some(request.audio_stream_index),
            format: request.format,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(
                    &media.path,
                    media.audio_streams.len(),
                    Some(request.audio_stream_index),
                ),
            ),
        };
        state.output_name.set_custom_path(request.output.clone());
        state
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

    pub fn output_directory(&self) -> &Path {
        self.output_name.directory()
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

    pub fn validation(&self, media: &MediaFile) -> Result<(), AudioExtractReadyError> {
        self.validated_request(media).map(|_| ())
    }

    pub fn validated_request(
        &self,
        media: &MediaFile,
    ) -> Result<AudioExtractRequest, AudioExtractReadyError> {
        let request = self
            .request(media)
            .ok_or(AudioExtractReadyError::NoAudioStreams)?;
        validate_audio_extract_request(&request, media).map_err(AudioExtractReadyError::Request)?;
        Ok(request)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioExtractReadyError {
    NoAudioStreams,
    Request(AudioExtractValidationError),
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use crate::{
        audio_extract::model::AudioExtractFormat,
        media::model::{AudioStream, MediaFile, MediaStream, StreamKind},
    };

    use super::*;

    pub(crate) fn media() -> MediaFile {
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
        let mut state = AudioExtractDraft::new(&media);
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

        let state = AudioExtractDraft::new(&media);

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-audio.mka")
        );
    }

    #[test]
    fn selection_uses_only_probed_absolute_audio_indexes() {
        let media = media();
        let mut state = AudioExtractDraft::new(&media);
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
        let mut state = AudioExtractDraft::new(&media);
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
    fn format_changes_authoritative_extension_and_preserves_custom_stem() {
        let media = media();
        let mut state = AudioExtractDraft::new(&media);
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
        let mut state = AudioExtractDraft::new(&media);
        state.set_custom_output_path(PathBuf::from("exports/voice.flac"));
        assert_eq!(state.output_path(), PathBuf::from("exports/voice.mka"));
        state.set_format(AudioExtractFormat::Flac);
        assert_eq!(state.output_path(), PathBuf::from("exports/voice.flac"));
    }
}

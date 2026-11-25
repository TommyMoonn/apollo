//! Editable operation configuration and typed request construction.

use super::{
    frame_index::FrameStep,
    model::{
        FrameCaptureFormat, FrameCaptureRequest, FramePosition, FrameTimestamp,
        FrameTimestampParseError,
    },
    validation::{
        FrameCaptureValidationError, validate_frame_capture_request, validate_frame_position,
        validate_frame_timestamp,
    },
};
use crate::media::{model::MediaFile, output_name::OutputNameState};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCaptureDraft {
    timestamp_text: String,
    position: FramePosition,
    step: FrameStep,
    format: FrameCaptureFormat,
    output_name: OutputNameState,
}

impl FrameCaptureDraft {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            timestamp_text: FrameTimestamp::ZERO.to_string(),
            position: FrameTimestamp::ZERO.into(),
            step: FrameStep::One,
            format: FrameCaptureFormat::Png,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(&media.path, FrameTimestamp::ZERO),
            ),
        }
    }

    pub fn from_request(media: &MediaFile, request: &FrameCaptureRequest) -> Self {
        let mut state = Self {
            timestamp_text: request.position.to_string(),
            position: request.position,
            step: FrameStep::One,
            format: request.format,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(&media.path, request.position.display_timestamp()),
            ),
        };
        state.output_name.set_custom_path(request.output.clone());
        state
    }

    pub fn timestamp_text(&self) -> &str {
        &self.timestamp_text
    }

    fn validated_timestamp(
        &self,
        media: &MediaFile,
    ) -> Result<FrameTimestamp, FrameCaptureReadyError> {
        let timestamp = FrameTimestamp::from_str(&self.timestamp_text)
            .map_err(FrameCaptureReadyError::Timestamp)?;
        validate_frame_timestamp(timestamp, media.duration)
            .map_err(FrameCaptureReadyError::Request)?;
        Ok(timestamp)
    }

    pub fn valid_timestamp(&self, media: &MediaFile) -> Option<FrameTimestamp> {
        self.validated_timestamp(media).ok()
    }

    pub fn valid_position(&self, media: &MediaFile) -> Option<FramePosition> {
        self.validated_timestamp(media).ok()?;
        validate_frame_position(self.position, media.duration)
            .is_ok()
            .then_some(self.position)
    }

    pub fn set_timestamp_text(&mut self, media: &MediaFile, timestamp: String) {
        self.timestamp_text = timestamp;
        if let Some(timestamp) = self.valid_timestamp(media) {
            self.position = timestamp.into();
            self.output_name
                .set_generated_stem(default_output_stem(&media.path, timestamp));
        }
    }

    pub fn set_position(&mut self, media: &MediaFile, position: FramePosition) {
        self.position = position;
        let timestamp = position.display_timestamp();
        self.timestamp_text = timestamp.to_string();
        self.output_name
            .set_generated_stem(default_output_stem(&media.path, timestamp));
    }

    pub const fn step(&self) -> FrameStep {
        self.step
    }

    pub fn set_step(&mut self, step: FrameStep) {
        self.step = step;
    }

    pub fn format(&self) -> FrameCaptureFormat {
        self.format
    }

    pub fn set_format(&mut self, format: FrameCaptureFormat) {
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

    pub fn request(&self, media: &MediaFile) -> Option<FrameCaptureRequest> {
        let request = self.request_from_text(media).ok()?;
        validate_frame_position(request.position, media.duration).ok()?;
        Some(request)
    }

    pub fn can_capture(&self, media: &MediaFile) -> bool {
        self.validation(media).is_ok()
    }

    pub fn validation(&self, media: &MediaFile) -> Result<(), FrameCaptureReadyError> {
        self.validated_request(media).map(|_| ())
    }

    pub fn validated_request(
        &self,
        media: &MediaFile,
    ) -> Result<FrameCaptureRequest, FrameCaptureReadyError> {
        let request = self.request_from_text(media)?;
        validate_frame_capture_request(&request, media).map_err(FrameCaptureReadyError::Request)?;
        Ok(request)
    }

    fn request_from_text(
        &self,
        media: &MediaFile,
    ) -> Result<FrameCaptureRequest, FrameCaptureReadyError> {
        self.validated_timestamp(media)?;
        Ok(FrameCaptureRequest {
            input: media.path.clone(),
            output: self.output_path(),
            position: self.position,
            format: self.format,
        })
    }
}

fn default_output_stem(input: &Path, timestamp: FrameTimestamp) -> OsString {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));
    let mut output = OsString::from(stem);
    output.push("-frame");
    output.push("-");
    output.push(timestamp.filename_component());
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameCaptureReadyError {
    Timestamp(FrameTimestampParseError),
    Request(FrameCaptureValidationError),
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{ffi::OsString, path::PathBuf, time::Duration};

    use crate::{
        frame_capture::{clipboard::copy_frame_action_eligible, model::FrameCaptureFormat},
        media::model::{MediaFile, MediaStream, StreamKind, VideoStream},
    };

    use super::*;

    pub(crate) fn media(duration: Option<Duration>) -> MediaFile {
        MediaFile {
            path: PathBuf::from("captures/recording.mkv"),
            duration,
            streams: vec![MediaStream {
                stream_index: 7,
                kind: StreamKind::Video,
                codec: Some("h264".into()),
            }],
            video_streams: vec![VideoStream {
                stream_index: 7,
                codec: Some("h264".into()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: Vec::new(),
        }
    }

    #[test]
    fn defaults_to_zero_png_and_timestamped_frame_suffix() {
        let media = media(Some(Duration::from_secs(10)));
        let state = FrameCaptureDraft::new(&media);
        assert_eq!(state.timestamp_text(), "00:00:00.000");
        assert_eq!(state.format(), FrameCaptureFormat::Png);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-frame-00-00-00-000.png")
        );
        assert!(state.can_capture(&media));
        assert_eq!(
            state
                .request(&media)
                .unwrap()
                .position
                .display_timestamp()
                .milliseconds(),
            0
        );
    }

    #[test]
    fn format_changes_extension_and_preserves_custom_stem() {
        let media = media(None);
        let mut state = FrameCaptureDraft::new(&media);
        state.set_output_filename_stem(OsString::from("chosen frame"));
        state.set_timestamp_text(&media, "00:00:04.250".into());
        state.set_format(FrameCaptureFormat::Jpeg);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/chosen frame.jpg")
        );
        state.set_format(FrameCaptureFormat::Png);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/chosen frame.png")
        );
        assert!(state.output_is_customized());
    }

    #[test]
    fn out_of_duration_timestamp_preserves_the_last_valid_generated_name() {
        let media = media(Some(Duration::from_secs(2)));
        let mut state = FrameCaptureDraft::new(&media);
        state.set_timestamp_text(&media, "00:00:01.500".into());
        let valid_output = state.output_path();

        state.set_timestamp_text(&media, "00:00:02.000".into());

        assert!(!state.can_capture(&media));
        assert_eq!(state.output_path(), valid_output);
    }

    #[test]
    fn invalid_capture_output_does_not_block_copying_a_valid_frame() {
        let media = media(Some(Duration::from_secs(10)));
        let mut state = FrameCaptureDraft::new(&media);
        state.set_output_filename_stem(OsString::new());

        assert!(!state.can_capture(&media));
        let position = state.valid_position(&media).unwrap();
        assert!(copy_frame_action_eligible(&media, position));
    }

    #[test]
    fn browse_path_preserves_directory_while_format_owns_extension() {
        let media = media(None);
        let mut state = FrameCaptureDraft::new(&media);
        state.set_custom_output_path(PathBuf::from("exports/still.jpg"));
        assert_eq!(state.output_path(), PathBuf::from("exports/still.png"));
        state.set_format(FrameCaptureFormat::Jpeg);
        assert_eq!(state.output_path(), PathBuf::from("exports/still.jpg"));
    }

    #[test]
    fn duration_and_missing_video_validation_control_primary_action() {
        let mut media = media(Some(Duration::from_millis(500)));
        let mut state = FrameCaptureDraft::new(&media);
        state.set_timestamp_text(&media, "00:00:00.501".into());
        assert!(!state.can_capture(&media));

        state.set_timestamp_text(&media, "00:00:00.000".into());
        media.video_streams.clear();
        assert!(!state.can_capture(&media));
    }
}

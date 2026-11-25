//! Editable operation configuration and typed request construction.

use super::{
    model::RemuxRequest,
    validation::{RemuxValidationError, validate_remux_request},
};
use crate::media::{container::MediaContainer, model::MediaFile, output_name::OutputNameState};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemuxDraft {
    container: MediaContainer,
    output_name: OutputNameState,
}

impl RemuxDraft {
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

    /// Construct and validate the request for a queue or batch submission.
    pub fn validated_request(
        &self,
        media: &MediaFile,
    ) -> Result<RemuxRequest, RemuxValidationError> {
        let request = self.request(media);
        validate_remux_request(&request, media)?;
        Ok(request)
    }

    pub fn validation(&self, media: &MediaFile) -> Result<(), RemuxValidationError> {
        self.validated_request(media).map(|_| ())
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

#[cfg(test)]
pub(crate) mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use crate::media::{
        container::MediaContainer,
        model::{MediaFile, MediaStream, StreamKind},
    };

    use super::*;

    pub(crate) fn media() -> MediaFile {
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

    pub(crate) fn media_with_streams(streams: Vec<MediaStream>) -> MediaFile {
        let mut media = media();
        media.streams = streams;
        media
    }

    #[test]
    fn default_state_uses_matroska_and_remuxed_output_name() {
        let media = media();
        let state = RemuxDraft::new(&media);

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
        let mut state = RemuxDraft::new(&media);

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
        let mut state = RemuxDraft::new(&media);
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
        let mut state = RemuxDraft::new(&media);
        state.set_custom_output_path(PathBuf::from("exports/final.mp4"));

        assert_eq!(state.output_path(), PathBuf::from("exports/final.mkv"));
        state.set_container(MediaContainer::Mp4);
        assert_eq!(state.output_path(), PathBuf::from("exports/final.mp4"));
    }

    #[test]
    fn validation_controls_primary_enablement() {
        let mut media = media();
        let mut state = RemuxDraft::new(&media);
        assert!(state.can_remux(&media));

        state.set_output_filename_stem(OsString::new());
        assert!(!state.can_remux(&media));

        state = RemuxDraft::new(&media);
        media.streams.clear();
        assert!(!state.can_remux(&media));
    }
}

//! Editable operation configuration and typed request construction.

use super::{
    model::{
        AudioMixRequest, LoudnessNormalization, LoudnessTargets, OutputContainer,
        SelectedAudioTrack, TrackGain,
    },
    validation::{AudioMixValidationError, validate_audio_mix_request},
};
use crate::media::{
    model::{AudioStream, MediaFile},
    output_name::OutputNameState,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

/// Pre-execution Audio Mix state. Process lifecycle remains owned by `AudioMixExecutor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMixDraft {
    selected_streams: BTreeSet<u32>,
    track_gains: BTreeMap<u32, TrackGain>,
    normalization_enabled: bool,
    normalization_targets: LoudnessTargets,
    container: OutputContainer,
    output_name: OutputNameState,
    ready_validation: AudioMixReadyValidationCache,
}

impl AudioMixDraft {
    /// New Audio Mix sessions begin with every probed audio stream selected and MP4 output.
    pub fn new(media: &MediaFile) -> Self {
        let mut state = Self::with_defaults(media);
        let _ = state.validation(media);
        state
    }

    fn with_defaults(media: &MediaFile) -> Self {
        Self {
            selected_streams: media
                .audio_streams
                .iter()
                .map(|stream| stream.stream_index)
                .collect(),
            track_gains: media
                .audio_streams
                .iter()
                .map(|stream| (stream.stream_index, TrackGain::default()))
                .collect(),
            normalization_enabled: false,
            normalization_targets: LoudnessTargets::default(),
            container: OutputContainer::Mp4,
            output_name: OutputNameState::new(&media.path, default_output_stem(&media.path)),
            ready_validation: AudioMixReadyValidationCache::default(),
        }
    }

    pub fn from_request(media: &MediaFile, request: &AudioMixRequest) -> Self {
        let mut state = Self::with_defaults(media);
        state.selected_streams = request
            .selected_tracks
            .iter()
            .map(|track| track.stream_index)
            .collect();
        for track in &request.selected_tracks {
            state.track_gains.insert(track.stream_index, track.gain);
        }
        match request.normalization {
            LoudnessNormalization::Off => state.normalization_enabled = false,
            LoudnessNormalization::Enabled(targets) => {
                state.normalization_enabled = true;
                state.normalization_targets = targets;
            }
        }
        state.container = request.container;
        state.output_name.set_custom_path(request.output.clone());
        state
    }

    pub fn container(&self) -> OutputContainer {
        self.container
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

    pub fn selected_count(&self) -> usize {
        self.selected_streams.len()
    }

    pub fn is_selected(&self, stream_index: u32) -> bool {
        self.selected_streams.contains(&stream_index)
    }

    pub fn track_gain(&self, stream_index: u32) -> Option<TrackGain> {
        self.track_gains.get(&stream_index).copied()
    }

    pub fn normalization_enabled(&self) -> bool {
        self.normalization_enabled
    }

    pub fn normalization_targets(&self) -> LoudnessTargets {
        self.normalization_targets
    }

    pub fn set_normalization_enabled(&mut self, enabled: bool) {
        if self.normalization_enabled == enabled {
            return;
        }
        self.normalization_enabled = enabled;
        self.invalidate_ready_validation();
    }

    pub fn set_normalization_targets(&mut self, targets: LoudnessTargets) -> bool {
        if !targets.integrated_loudness_is_valid()
            || !targets.maximum_true_peak_is_valid()
            || !targets.loudness_range_is_valid()
        {
            return false;
        }

        if self.normalization_targets != targets {
            self.normalization_targets = targets;
            self.invalidate_ready_validation();
        }
        true
    }

    pub fn reset_normalization_targets(&mut self) {
        let defaults = LoudnessTargets::default();
        if self.normalization_targets != defaults {
            self.normalization_targets = defaults;
            self.invalidate_ready_validation();
        }
    }

    pub fn can_mix(&mut self, media: &MediaFile) -> bool {
        self.validation(media).is_ok()
    }

    /// Construct and validate the request for a queue or batch submission.
    pub fn validated_request(
        &self,
        media: &MediaFile,
    ) -> Result<AudioMixRequest, AudioMixValidationError> {
        let request = self.request(media);
        validate_audio_mix_request(&request, media)?;
        Ok(request)
    }

    pub fn validation(&mut self, media: &MediaFile) -> &Result<(), AudioMixValidationError> {
        let has_video = !media.video_streams.is_empty();
        let source_changed = self.ready_validation.source_has_video != has_video
            || self.ready_validation.source_audio != media.audio_streams;
        if self.ready_validation.dirty || source_changed {
            if source_changed {
                self.ready_validation
                    .source_audio
                    .clone_from(&media.audio_streams);
                self.ready_validation.source_has_video = has_video;
            }
            self.ready_validation.result = self.validated_request(media).map(|_| ());
            self.ready_validation.dirty = false;
            #[cfg(test)]
            {
                self.ready_validation.rebuild_count += 1;
            }
        }
        &self.ready_validation.result
    }

    fn invalidate_ready_validation(&mut self) {
        self.ready_validation.dirty = true;
    }

    #[cfg(test)]
    pub(crate) fn ready_validation_rebuild_count(&self) -> usize {
        self.ready_validation.rebuild_count
    }

    /// Toggle only a stream that belongs to the current probed media.
    pub fn toggle_stream(&mut self, media: &MediaFile, stream_index: u32) -> bool {
        if !media
            .audio_streams
            .iter()
            .any(|stream| stream.stream_index == stream_index)
        {
            return false;
        }

        if !self.selected_streams.insert(stream_index) {
            self.selected_streams.remove(&stream_index);
        }
        self.invalidate_ready_validation();
        true
    }

    pub fn select_all(&mut self, media: &MediaFile) {
        let selected_streams = media
            .audio_streams
            .iter()
            .map(|stream| stream.stream_index)
            .collect();
        if self.selected_streams != selected_streams {
            self.selected_streams = selected_streams;
            self.invalidate_ready_validation();
        }
    }

    pub fn clear(&mut self) {
        if !self.selected_streams.is_empty() {
            self.selected_streams.clear();
            self.invalidate_ready_validation();
        }
    }

    /// Change one current stream's gain without affecting its selection or any sibling track.
    pub fn set_track_gain(
        &mut self,
        media: &MediaFile,
        stream_index: u32,
        gain: TrackGain,
    ) -> bool {
        if !gain.is_valid()
            || !media
                .audio_streams
                .iter()
                .any(|stream| stream.stream_index == stream_index)
        {
            return false;
        }

        if self.track_gains.get(&stream_index).copied() != Some(gain) {
            self.track_gains.insert(stream_index, gain);
            self.invalidate_ready_validation();
        }
        true
    }

    pub fn reset_track_gains(&mut self) {
        let mut changed = false;
        for gain in self.track_gains.values_mut() {
            changed |= *gain != TrackGain::ZERO;
            *gain = TrackGain::ZERO;
        }
        if changed {
            self.invalidate_ready_validation();
        }
    }

    /// Change container while preserving a deliberately customized output path.
    pub fn set_container(&mut self, media: &MediaFile, container: OutputContainer) {
        if self.container == container {
            return;
        }

        self.container = container;
        self.output_name
            .set_generated_stem(default_output_stem(&media.path));
        self.invalidate_ready_validation();
    }

    pub fn set_output_filename_stem(&mut self, stem: OsString) {
        if self.output_name.stem() == stem.as_os_str() {
            return;
        }
        self.output_name.set_stem(stem);
        self.invalidate_ready_validation();
    }

    pub fn set_custom_output_path(&mut self, output: PathBuf) {
        self.output_name.set_custom_path(output);
        self.invalidate_ready_validation();
    }

    /// Build the current request without starting work. Stream order follows probe/source order.
    pub fn request(&self, media: &MediaFile) -> AudioMixRequest {
        AudioMixRequest {
            input: media.path.clone(),
            output: self.output_path(),
            selected_tracks: media
                .audio_streams
                .iter()
                .filter(|stream| self.selected_streams.contains(&stream.stream_index))
                .map(|stream| {
                    SelectedAudioTrack::new(
                        stream.stream_index,
                        self.track_gains
                            .get(&stream.stream_index)
                            .copied()
                            .unwrap_or_default(),
                    )
                })
                .collect(),
            normalization: if self.normalization_enabled {
                LoudnessNormalization::Enabled(self.normalization_targets)
            } else {
                LoudnessNormalization::Off
            },
            container: self.container,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioMixReadyValidationCache {
    dirty: bool,
    source_audio: Vec<AudioStream>,
    source_has_video: bool,
    result: Result<(), AudioMixValidationError>,
    #[cfg(test)]
    rebuild_count: usize,
}

impl Default for AudioMixReadyValidationCache {
    fn default() -> Self {
        Self {
            dirty: true,
            source_audio: Vec::new(),
            source_has_video: false,
            result: Ok(()),
            #[cfg(test)]
            rebuild_count: 0,
        }
    }
}

fn default_output_stem(input: &Path) -> OsString {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));

    let mut output_stem = OsString::from(stem);
    output_stem.push("-mixed");
    output_stem
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
        time::Duration,
    };

    use crate::media::model::{AudioStream, MediaFile, VideoStream};

    use crate::audio_mix::model::{
        LoudnessNormalization, LoudnessTargets, OutputContainer, SelectedAudioTrack, TrackGain,
    };
    use crate::audio_mix::validation::AudioMixValidationError;

    use super::*;

    pub(crate) fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("captures/recording.mkv"),
            duration: Some(Duration::from_secs(6136)),
            streams: Vec::new(),
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(3840),
                height: Some(2160),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: vec![
                AudioStream {
                    stream_index: 1,
                    codec: Some("aac".to_owned()),
                    channels: Some(2),
                    channel_layout: Some("stereo".to_owned()),
                    title: Some("Desktop Audio".to_owned()),
                    language: Some("eng".to_owned()),
                },
                AudioStream {
                    stream_index: 4,
                    codec: Some("aac".to_owned()),
                    channels: Some(1),
                    channel_layout: Some("mono".to_owned()),
                    title: Some("Microphone".to_owned()),
                    language: None,
                },
                AudioStream {
                    stream_index: 9,
                    codec: Some("opus".to_owned()),
                    channels: Some(2),
                    channel_layout: None,
                    title: None,
                    language: Some("jpn".to_owned()),
                },
            ],
        }
    }

    #[test]
    fn initial_policy_selects_all_probed_audio_streams() {
        let media = media();
        let state = AudioMixDraft::new(&media);

        assert_eq!(state.selected_count(), 3);
        assert!(state.is_selected(1));
        assert!(state.is_selected(4));
        assert!(state.is_selected(9));
    }

    #[test]
    fn cached_ready_validation_tracks_invalid_and_corrected_requests() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        assert!(state.can_mix(&media));
        state.clear();
        assert!(!state.can_mix(&media));
        state.select_all(&media);
        assert!(state.can_mix(&media));
        assert!(
            media
                .audio_streams
                .iter()
                .all(|track| state.is_selected(track.stream_index))
        );
        state.set_output_filename_stem(OsString::from(""));
        assert!(!state.can_mix(&media));
        state.set_custom_output_path(PathBuf::from("exports/final-mix.mp4"));
        assert!(state.can_mix(&media));
    }

    #[test]
    fn replacement_source_builds_source_bound_ready_validation() {
        let original = media();
        let mut original_state = AudioMixDraft::new(&original);
        assert!(original_state.can_mix(&original));

        let mut replacement = original.clone();
        replacement.path = PathBuf::from("captures/replacement.mkv");
        replacement.video_streams.clear();
        assert!(!original_state.can_mix(&replacement));
        assert_eq!(
            original_state.validation(&replacement).as_ref(),
            Err(&AudioMixValidationError::MissingVideoStream)
        );
        assert!(original_state.can_mix(&original));
        replacement = original.clone();
        replacement.audio_streams[0].channels = Some(6);
        assert!(matches!(
            original_state.validation(&replacement),
            Err(AudioMixValidationError::UnsupportedChannelLayout { .. })
        ));
        assert!(original_state.can_mix(&original));
    }

    #[test]
    fn toggle_one_track_uses_absolute_stream_index() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);

        assert!(state.toggle_stream(&media, 4));
        assert!(!state.is_selected(4));
        assert_eq!(state.selected_count(), 2);
        assert!(!state.toggle_stream(&media, 99));
        assert_eq!(state.selected_count(), 2);
    }

    #[test]
    fn ready_validation_ignores_destination_filesystem_state() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        state.set_container(&media, OutputContainer::Mkv);

        state.set_custom_output_path(media.path.clone());
        assert!(state.validation(&media).is_ok());
        assert!(state.can_mix(&media));

        state.set_custom_output_path(PathBuf::from("missing-parent/output.mkv"));
        assert!(state.validation(&media).is_ok());
        assert!(state.can_mix(&media));
    }

    #[test]
    fn mkv_selection_updates_generated_default_extension() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);

        state.set_container(&media, OutputContainer::Mkv);

        assert_eq!(state.container(), OutputContainer::Mkv);
        assert_eq!(
            state.output_path(),
            Path::new("captures/recording-mixed.mkv")
        );
        assert!(!state.output_is_customized());
    }

    #[test]
    fn mp4_selection_restores_generated_mp4_extension() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        state.set_container(&media, OutputContainer::Mkv);

        state.set_container(&media, OutputContainer::Mp4);

        assert_eq!(state.container(), OutputContainer::Mp4);
        assert_eq!(
            state.output_path(),
            Path::new("captures/recording-mixed.mp4")
        );
    }

    #[test]
    fn generated_default_filename_uses_mixed_suffix() {
        let media = media();
        let state = AudioMixDraft::new(&media);

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-mixed.mp4")
        );
        assert!(!state.output_is_customized());
    }

    #[test]
    fn custom_output_stem_is_preserved_across_container_changes() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        state.set_custom_output_path(PathBuf::from("exports/final-audio.mkv"));

        assert_eq!(
            state.output_path(),
            PathBuf::from("exports/final-audio.mp4")
        );
        state.set_output_filename_stem(OsString::from("renamed mix"));
        assert_eq!(
            state.output_path(),
            PathBuf::from("exports/renamed mix.mp4")
        );

        state.set_container(&media, OutputContainer::Mkv);

        assert_eq!(
            state.output_path(),
            PathBuf::from("exports/renamed mix.mkv")
        );
        assert_eq!(state.output_filename_stem(), OsString::from("renamed mix"));
        assert!(state.output_is_customized());
    }

    #[test]
    fn edited_filename_stem_preserves_unicode_spaces_and_authoritative_extension() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);

        state.set_output_filename_stem(OsString::from("final mix 日本語"));

        assert!(state.output_is_customized());
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/final mix 日本語.mp4")
        );
        assert_eq!(
            state.request(&media).output,
            PathBuf::from("captures/final mix 日本語.mp4")
        );

        state.set_container(&media, OutputContainer::Mkv);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/final mix 日本語.mkv")
        );
    }

    #[test]
    fn windows_reserved_device_stem_disables_mix() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);

        state.set_output_filename_stem(OsString::from("cOn"));

        assert!(!state.can_mix(&media));
        assert!(state.request(&media).output.as_os_str().is_empty());
    }

    #[test]
    fn request_preserves_source_order_for_selected_absolute_indexes() {
        let mut media = media();
        media.audio_streams.swap(0, 2);
        let mut state = AudioMixDraft::new(&media);
        assert!(state.set_track_gain(&media, 9, TrackGain::from_tenths_db(-60)));
        state.toggle_stream(&media, 4);

        let request = state.request(&media);

        assert_eq!(
            request.selected_tracks,
            vec![
                SelectedAudioTrack::new(9, TrackGain::from_tenths_db(-60)),
                SelectedAudioTrack::with_default_gain(1),
            ]
        );
        assert_eq!(request.input, media.path);
        assert_eq!(
            request.output,
            PathBuf::from("captures/recording-mixed.mp4")
        );
    }

    #[test]
    fn every_probed_track_starts_at_zero_gain_independent_of_selection() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        let initial_gains = state.track_gains.clone();

        assert_eq!(initial_gains.len(), media.audio_streams.len());
        assert!(initial_gains.values().all(|gain| *gain == TrackGain::ZERO));

        state.clear();
        assert_eq!(state.track_gains, initial_gains);
        state.select_all(&media);
        assert_eq!(state.track_gains, initial_gains);
    }

    #[test]
    fn editing_one_track_gain_is_isolated_and_reaches_the_request() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);

        assert!(state.set_track_gain(&media, 4, TrackGain::from_tenths_db(-55)));

        assert_eq!(state.track_gain(1), Some(TrackGain::ZERO));
        assert_eq!(state.track_gain(4), Some(TrackGain::from_tenths_db(-55)));
        assert_eq!(state.track_gain(9), Some(TrackGain::ZERO));
        assert_eq!(state.selected_count(), 3);
        assert_eq!(
            state.request(&media).selected_tracks,
            vec![
                SelectedAudioTrack::with_default_gain(1),
                SelectedAudioTrack::new(4, TrackGain::from_tenths_db(-55)),
                SelectedAudioTrack::with_default_gain(9),
            ]
        );
    }

    #[test]
    fn selection_operations_preserve_stored_track_gains() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        let adjusted = TrackGain::from_tenths_db(35);
        assert!(state.set_track_gain(&media, 4, adjusted));

        assert!(state.toggle_stream(&media, 4));
        assert!(!state.is_selected(4));
        assert_eq!(state.track_gain(4), Some(adjusted));
        assert!(state.toggle_stream(&media, 4));
        assert!(state.is_selected(4));
        assert_eq!(state.track_gain(4), Some(adjusted));

        state.clear();
        assert_eq!(state.track_gain(4), Some(adjusted));
        state.select_all(&media);
        assert_eq!(state.track_gain(4), Some(adjusted));
    }

    #[test]
    fn reset_gains_changes_no_track_selection() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        assert!(state.set_track_gain(&media, 1, TrackGain::from_tenths_db(-100)));
        assert!(state.set_track_gain(&media, 9, TrackGain::from_tenths_db(120)));
        assert!(state.toggle_stream(&media, 4));
        let selected_before = state.selected_streams.clone();

        state.reset_track_gains();

        assert_eq!(state.selected_streams, selected_before);
        assert!(
            state
                .track_gains
                .values()
                .all(|gain| *gain == TrackGain::ZERO)
        );
    }

    #[test]
    fn gain_state_rejects_unknown_streams_and_out_of_range_values() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);

        assert!(!state.set_track_gain(&media, 99, TrackGain::ZERO));
        assert!(!state.set_track_gain(&media, 1, TrackGain::from_tenths_db(121)));
        assert_eq!(state.track_gain(1), Some(TrackGain::ZERO));
    }

    #[test]
    fn new_session_starts_with_normalization_off_and_default_targets() {
        let state = AudioMixDraft::new(&media());

        assert!(!state.normalization_enabled());
        assert_eq!(state.normalization_targets(), LoudnessTargets::default());
    }

    #[test]
    fn normalization_toggle_preserves_edited_targets() {
        let mut state = AudioMixDraft::new(&media());
        let targets = LoudnessTargets::from_tenths(-235, -7, 185);
        assert!(state.set_normalization_targets(targets));

        state.set_normalization_enabled(true);
        state.set_normalization_enabled(false);
        state.set_normalization_enabled(true);

        assert!(state.normalization_enabled());
        assert_eq!(state.normalization_targets(), targets);
    }

    #[test]
    fn normalization_request_matches_visible_enabled_targets() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        let targets = LoudnessTargets::from_tenths(-205, -12, 75);
        assert!(state.set_normalization_targets(targets));

        assert_eq!(
            state.request(&media).normalization,
            LoudnessNormalization::Off
        );
        state.set_normalization_enabled(true);
        assert_eq!(
            state.request(&media).normalization,
            LoudnessNormalization::Enabled(targets)
        );
    }

    #[test]
    fn normalization_edits_and_reset_leave_unrelated_audio_mix_state_unchanged() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);
        state.toggle_stream(&media, 4);
        assert!(state.set_track_gain(&media, 1, TrackGain::from_tenths_db(-55)));
        state.set_container(&media, OutputContainer::Mkv);
        state.set_output_filename_stem(OsString::from("custom mix"));
        let unrelated_before = (
            state.selected_streams.clone(),
            state.track_gains.clone(),
            state.container,
            state.output_name.clone(),
        );

        state.set_normalization_enabled(true);
        assert!(state.set_normalization_targets(LoudnessTargets::from_tenths(-220, -8, 145)));
        state.reset_normalization_targets();

        assert_eq!(state.normalization_targets(), LoudnessTargets::default());
        assert!(state.normalization_enabled());
        assert_eq!(
            (
                state.selected_streams,
                state.track_gains,
                state.container,
                state.output_name,
            ),
            unrelated_before
        );
    }

    #[test]
    fn normalization_state_rejects_targets_outside_domain_bounds() {
        let mut state = AudioMixDraft::new(&media());

        assert!(!state.set_normalization_targets(LoudnessTargets::from_tenths(-701, -15, 110)));
        assert!(!state.set_normalization_targets(LoudnessTargets::from_tenths(-160, 1, 110)));
        assert!(!state.set_normalization_targets(LoudnessTargets::from_tenths(-160, -15, 501)));
        assert_eq!(state.normalization_targets(), LoudnessTargets::default());
    }
}

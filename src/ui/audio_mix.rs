use std::{
    collections::{BTreeSet, HashMap},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Duration,
};

use egui::{Align, Label, Layout, RichText, ScrollArea, Sides, TextStyle, Ui};

use crate::{
    audio_mix::{
        model::{AudioMixRequest, OutputContainer},
        validation::{AudioMixValidationError, validate_audio_mix_request},
    },
    media::{
        model::{AudioStream, MediaFile},
        output::MediaOutputError,
    },
};

use super::{fonts, layout, output_name::OutputNameState, theme, widgets};

const UNTITLED_AUDIO: &str = "Untitled audio";
const AUDIO_TRACKS_LABEL: &str = "Audio tracks";
const OUTPUT_LABEL: &str = "Output";
const BACK_LABEL: &str = "← Back";
const SELECT_ALL_LABEL: &str = "Select all";
const CLEAR_LABEL: &str = "Clear";
const BROWSE_LABEL: &str = "Browse";
const MIX_LABEL: &str = "Mix selected tracks";
const CONTAINER_OPTIONS: [&str; 2] = ["MP4", "MKV"];

/// Pre-execution Audio Mix state. Process lifecycle remains owned by `AudioMixExecutor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMixSelectionState {
    selected_streams: BTreeSet<u32>,
    container: OutputContainer,
    output_name: OutputNameState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioMixSelectionAction {
    #[default]
    None,
    Back,
    BrowseOutput,
    MixRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackPresentation {
    stream_index: u32,
    title: String,
    metadata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackIdentity {
    stream_index: u32,
    title: String,
    channel: Option<String>,
    language: Option<String>,
    codec: Option<String>,
}

impl AudioMixSelectionState {
    /// New Audio Mix sessions begin with every probed audio stream selected and MP4 output.
    pub fn new(media: &MediaFile) -> Self {
        Self {
            selected_streams: media
                .audio_streams
                .iter()
                .map(|stream| stream.stream_index)
                .collect(),
            container: OutputContainer::Mp4,
            output_name: OutputNameState::new(&media.path, default_output_stem(&media.path)),
        }
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

    pub fn can_mix(&self, media: &MediaFile) -> bool {
        self.validation(media).is_ok()
    }

    fn validation(&self, media: &MediaFile) -> Result<(), AudioMixValidationError> {
        validate_audio_mix_request(&self.request(media), media)
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
        true
    }

    pub fn select_all(&mut self, media: &MediaFile) {
        self.selected_streams = media
            .audio_streams
            .iter()
            .map(|stream| stream.stream_index)
            .collect();
    }

    pub fn clear(&mut self) {
        self.selected_streams.clear();
    }

    /// Change container while preserving a deliberately customized output path.
    pub fn set_container(&mut self, media: &MediaFile, container: OutputContainer) {
        if self.container == container {
            return;
        }

        self.container = container;
        self.output_name
            .set_generated_stem(default_output_stem(&media.path));
    }

    pub fn set_output_filename_stem(&mut self, stem: OsString) {
        self.output_name.set_stem(stem);
    }

    pub fn set_custom_output_path(&mut self, output: PathBuf) {
        self.output_name.set_custom_path(output);
    }

    /// Build the Phase 24 request without starting work. Stream order follows probe/source order.
    pub fn request(&self, media: &MediaFile) -> AudioMixRequest {
        AudioMixRequest {
            input: media.path.clone(),
            output: self.output_path(),
            selected_streams: media
                .audio_streams
                .iter()
                .filter_map(|stream| {
                    self.selected_streams
                        .contains(&stream.stream_index)
                        .then_some(stream.stream_index)
                })
                .collect(),
            container: self.container,
        }
    }
}

/// Render the complete pre-execution Audio Mix configuration surface.
///
/// The caller owns source acquisition and execution orchestration. This surface only mutates
/// selection, container, and output-path state and emits user intent for those responsibilities.
pub fn show(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut AudioMixSelectionState,
) -> AudioMixSelectionAction {
    let mut action = AudioMixSelectionAction::None;
    let tracks = track_presentations(media);
    let list_content_height = tracks.len() as f32 * theme::SELECTABLE_ROW_MIN_HEIGHT;
    let has_validation_feedback = state
        .validation(media)
        .as_ref()
        .err()
        .and_then(audio_mix_ready_feedback)
        .is_some();
    let plan = layout::task_surface_plan(
        ui.available_height(),
        audio_mix_fixed_flow_height(ui, media, tracks.is_empty(), has_validation_feedback),
        list_content_height,
    );

    layout::task_surface(ui, plan.overflow, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;

        if show_source_header(ui, media) {
            action = AudioMixSelectionAction::Back;
        }
        ui.add_space(theme::SPACING.section);

        let _ = widgets::section_label(ui, AUDIO_TRACKS_LABEL);
        ui.add_space(theme::SPACING.compact);
        show_track_list(ui, media, state, &tracks, plan.list.max_height);
        ui.add_space(theme::INLINE_ACTION_GAP);
        show_track_actions(ui, media, state);
        ui.add_space(theme::SPACING.section);

        let _ = widgets::section_label(ui, OUTPUT_LABEL);
        ui.add_space(theme::SPACING.compact);
        let output_action = show_output_row(ui, media, state);
        if output_action != AudioMixSelectionAction::None {
            action = output_action;
        }

        let validation = state.validation(media);
        if let Some(feedback) = validation.as_ref().err().and_then(audio_mix_ready_feedback) {
            ui.add_space(theme::SPACING.tight);
            let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
        }

        ui.add_space(theme::SPACING.related);
        let response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::primary_button_enabled(ui, MIX_LABEL, validation.is_ok())
            })
            .inner;
        if response.clicked() {
            action = AudioMixSelectionAction::MixRequested;
        }
    });

    action
}

fn show_source_header(ui: &mut Ui, media: &MediaFile) -> bool {
    let file_name = source_file_name(&media.path);
    let metadata = source_metadata(media);
    let metadata_gap = if metadata.is_some() {
        theme::SPACING.tight
    } else {
        0.0
    };
    let header_height = source_header_height(ui, metadata.is_some());

    let (_, back_response) = Sides::new()
        .height(header_height.max(theme::COMPACT_CONTROL_HEIGHT))
        .spacing(theme::SOURCE_HEADER_GAP)
        .shrink_left()
        .truncate()
        .show(
            ui,
            |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = metadata_gap;
                    ui.add(
                        Label::new(RichText::new(file_name).text_style(TextStyle::Body))
                            .truncate()
                            .selectable(false),
                    );
                    if let Some(metadata) = metadata {
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
                });
            },
            |ui| widgets::plain_button(ui, BACK_LABEL),
        );

    back_response.clicked()
}

fn show_track_list(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut AudioMixSelectionState,
    tracks: &[TrackPresentation],
    list_height: f32,
) {
    widgets::separator(ui);

    if tracks.is_empty() {
        widgets::separator(ui);
        return;
    }

    ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for track in tracks {
                let response = widgets::selectable_row(
                    ui,
                    &track.title,
                    track.metadata.as_deref(),
                    state.is_selected(track.stream_index),
                    widgets::SelectionIndicator::Checkbox,
                    theme::SELECTABLE_ROW_MIN_HEIGHT,
                );

                if response.clicked() {
                    state.toggle_stream(media, track.stream_index);
                }
            }
        });
}

fn audio_mix_fixed_flow_height(
    ui: &Ui,
    media: &MediaFile,
    track_list_empty: bool,
    has_validation_feedback: bool,
) -> f32 {
    let section_label_height = ui.text_style_height(&TextStyle::Small);
    let source_header_height = source_header_height(ui, source_metadata(media).is_some());
    let output_row_height = widgets::output_filename_row_plan(
        ui,
        Some(widgets::select_menu_intrinsic_width(ui, &CONTAINER_OPTIONS)),
        BROWSE_LABEL,
    )
    .flow_height();
    let track_list_chrome = if track_list_empty {
        theme::SEPARATOR_WIDTH * 2.0
    } else {
        theme::SEPARATOR_WIDTH
    };
    let feedback_height = if has_validation_feedback {
        theme::SPACING.tight + ui.text_style_height(&TextStyle::Body)
    } else {
        0.0
    };

    source_header_height
        + theme::SPACING.section
        + section_label_height
        + theme::SPACING.compact
        + track_list_chrome
        + theme::INLINE_ACTION_GAP
        + theme::COMPACT_CONTROL_HEIGHT
        + theme::SPACING.section
        + section_label_height
        + theme::SPACING.compact
        + output_row_height
        + feedback_height
        + theme::SPACING.related
        + theme::PRIMARY_CONTROL_HEIGHT
}

fn source_header_height(ui: &Ui, has_metadata: bool) -> f32 {
    let body_height = ui.text_style_height(&TextStyle::Body);
    let metadata_height = if has_metadata {
        theme::SPACING.tight + ui.text_style_height(&fonts::technical_text_style())
    } else {
        0.0
    };

    (body_height + metadata_height).max(theme::COMPACT_CONTROL_HEIGHT)
}

fn show_track_actions(ui: &mut Ui, media: &MediaFile, state: &mut AudioMixSelectionState) {
    let selected_count = state.selected_count();

    Sides::new().height(theme::COMPACT_CONTROL_HEIGHT).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::INLINE_ACTION_GAP;
            if widgets::plain_button(ui, SELECT_ALL_LABEL).clicked() {
                state.select_all(media);
            }
            if widgets::plain_button(ui, CLEAR_LABEL).clicked() {
                state.clear();
            }
        },
        |ui| {
            ui.label(
                RichText::new(format!("{selected_count} selected"))
                    .text_style(fonts::technical_text_style())
                    .color(theme::SECONDARY_FAINT),
            );
        },
    );
}

fn audio_mix_ready_feedback(error: &AudioMixValidationError) -> Option<&'static str> {
    match error {
        AudioMixValidationError::OutputExtensionMismatch { actual: None, .. }
        | AudioMixValidationError::Output(MediaOutputError::InvalidOutputPath(_)) => {
            Some("Enter a usable output filename.")
        }
        AudioMixValidationError::OutputExtensionMismatch { .. } => {
            Some("Output filename must match the selected container.")
        }
        AudioMixValidationError::Output(MediaOutputError::SameInputAndOutput(_)) => {
            Some("Output must be different from the source file.")
        }
        AudioMixValidationError::Output(MediaOutputError::OutputParentUnavailable { .. })
        | AudioMixValidationError::Output(MediaOutputError::OutputParentNotDirectory(_)) => {
            Some("Choose an available output folder.")
        }
        AudioMixValidationError::Output(MediaOutputError::OutputAlreadyExists(_)) => {
            Some("Choose a different output filename or destination.")
        }
        _ => None,
    }
}

fn show_output_row(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut AudioMixSelectionState,
) -> AudioMixSelectionAction {
    let mut requested_container = None;
    let mut output_stem = state.output_filename_stem().to_string_lossy().into_owned();
    let selector_width = widgets::select_menu_intrinsic_width(ui, &CONTAINER_OPTIONS);

    let row = widgets::output_filename_row_with_leading(
        ui,
        &mut output_stem,
        state.container().extension(),
        BROWSE_LABEL,
        selector_width,
        |ui| {
            widgets::select_menu(
                ui,
                container_label(state.container()),
                &CONTAINER_OPTIONS,
                |ui| {
                    if widgets::select_menu_option(
                        ui,
                        "MP4",
                        state.container() == OutputContainer::Mp4,
                    )
                    .clicked()
                    {
                        requested_container = Some(OutputContainer::Mp4);
                        ui.close();
                    }
                    if widgets::select_menu_option(
                        ui,
                        "MKV",
                        state.container() == OutputContainer::Mkv,
                    )
                    .clicked()
                    {
                        requested_container = Some(OutputContainer::Mkv);
                        ui.close();
                    }
                },
            );
        },
    );

    if row.filename.changed() {
        state.set_output_filename_stem(OsString::from(output_stem));
    }
    if let Some(container) = requested_container {
        state.set_container(media, container);
    }

    if row.browse.clicked() {
        AudioMixSelectionAction::BrowseOutput
    } else {
        AudioMixSelectionAction::None
    }
}

fn source_file_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn source_metadata(media: &MediaFile) -> Option<String> {
    let mut parts = Vec::new();

    if let Some(video) = media.video_streams.first()
        && let (Some(width), Some(height)) = (video.width, video.height)
    {
        parts.push(format!("{width}×{height}"));
    }

    if let Some(duration) = media.duration {
        parts.push(format_duration(duration));
    }

    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn track_presentations(media: &MediaFile) -> Vec<TrackPresentation> {
    let identities = media
        .audio_streams
        .iter()
        .map(|stream| TrackIdentity {
            stream_index: stream.stream_index,
            title: track_title(stream),
            channel: channel_description(stream),
            language: nonempty(stream.language.as_deref()).map(str::to_owned),
            codec: nonempty(stream.codec.as_deref()).map(str::to_owned),
        })
        .collect::<Vec<_>>();

    let mut title_channel_groups: HashMap<(String, Option<String>), Vec<usize>> = HashMap::new();
    for (index, identity) in identities.iter().enumerate() {
        title_channel_groups
            .entry((
                identity.title.clone(),
                normalized_key(identity.channel.as_deref()),
            ))
            .or_default()
            .push(index);
    }

    let mut show_language = vec![false; identities.len()];
    for indexes in title_channel_groups
        .values()
        .filter(|indexes| indexes.len() > 1)
    {
        if group_values_differ(indexes, |index| {
            normalized_key(identities[index].language.as_deref())
        }) {
            for &index in indexes {
                show_language[index] = true;
            }
        }
    }

    let mut residual_groups: HashMap<(String, Option<String>, Option<String>), Vec<usize>> =
        HashMap::new();
    for (index, identity) in identities.iter().enumerate() {
        residual_groups
            .entry((
                identity.title.clone(),
                normalized_key(identity.channel.as_deref()),
                show_language[index]
                    .then(|| normalized_key(identity.language.as_deref()))
                    .flatten(),
            ))
            .or_default()
            .push(index);
    }

    let mut show_codec = vec![false; identities.len()];
    for indexes in residual_groups.values().filter(|indexes| indexes.len() > 1) {
        if group_values_differ(indexes, |index| {
            normalized_key(identities[index].codec.as_deref())
        }) {
            for &index in indexes {
                show_codec[index] = true;
            }
        }
    }

    identities
        .into_iter()
        .enumerate()
        .map(|(index, identity)| {
            let metadata = track_metadata(&identity, show_language[index], show_codec[index]);
            TrackPresentation {
                stream_index: identity.stream_index,
                title: identity.title,
                metadata,
            }
        })
        .collect()
}

fn track_title(stream: &AudioStream) -> String {
    stream
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(UNTITLED_AUDIO)
        .to_owned()
}

fn track_metadata(
    identity: &TrackIdentity,
    show_language: bool,
    show_codec: bool,
) -> Option<String> {
    let mut parts = Vec::new();

    if let Some(channel) = &identity.channel {
        parts.push(channel.clone());
    }

    if show_language && let Some(language) = &identity.language {
        parts.push(language.clone());
    }
    if show_codec && let Some(codec) = &identity.codec {
        parts.push(codec.clone());
    }

    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn group_values_differ(
    indexes: &[usize],
    mut value_for: impl FnMut(usize) -> Option<String>,
) -> bool {
    let Some((&first, rest)) = indexes.split_first() else {
        return false;
    };
    let first_value = value_for(first);

    rest.iter().any(|&index| value_for(index) != first_value)
}

fn normalized_key(value: Option<&str>) -> Option<String> {
    nonempty(value).map(|value| value.to_ascii_lowercase())
}

fn channel_description(stream: &AudioStream) -> Option<String> {
    if let Some(layout) = nonempty(stream.channel_layout.as_deref()) {
        return Some(match layout.to_ascii_lowercase().as_str() {
            "mono" => "Mono".to_owned(),
            "stereo" => "Stereo".to_owned(),
            _ => layout.to_owned(),
        });
    }

    stream.channels.map(|channels| match channels {
        1 => "Mono".to_owned(),
        2 => "Stereo".to_owned(),
        channels => format!("{channels} ch"),
    })
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
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

fn container_label(container: OutputContainer) -> &'static str {
    match container {
        OutputContainer::Mp4 => "MP4",
        OutputContainer::Mkv => "MKV",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
        time::Duration,
    };

    use crate::{
        audio_mix::validation::AudioMixValidationError,
        media::{
            model::{AudioStream, MediaFile, VideoStream},
            output::MediaOutputError,
        },
    };

    use super::{
        AudioMixSelectionState, UNTITLED_AUDIO, audio_mix_fixed_flow_height,
        audio_mix_ready_feedback, source_metadata, track_presentations,
    };
    use crate::audio_mix::model::OutputContainer;
    use crate::ui::{layout, theme};

    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("captures/recording.mkv"),
            duration: Some(Duration::from_secs(6136)),
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(3840),
                height: Some(2160),
                frame_rate: None,
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
        let state = AudioMixSelectionState::new(&media);

        assert_eq!(state.selected_count(), 3);
        assert!(state.is_selected(1));
        assert!(state.is_selected(4));
        assert!(state.is_selected(9));
    }

    #[test]
    fn valid_request_enables_primary_action() {
        let mut media = media();
        media.path = PathBuf::from("audio-mix-ready-source.mkv");
        let state = AudioMixSelectionState::new(&media);

        assert!(state.can_mix(&media));
    }

    #[test]
    fn toggle_one_track_uses_absolute_stream_index() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);

        assert!(state.toggle_stream(&media, 4));
        assert!(!state.is_selected(4));
        assert_eq!(state.selected_count(), 2);
        assert!(!state.toggle_stream(&media, 99));
        assert_eq!(state.selected_count(), 2);
    }

    #[test]
    fn select_all_restores_every_probed_audio_stream() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);
        state.clear();

        state.select_all(&media);

        assert_eq!(state.selected_count(), media.audio_streams.len());
    }

    #[test]
    fn clear_all_disables_primary_action() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);

        state.clear();

        assert_eq!(state.selected_count(), 0);
        assert!(!state.can_mix(&media));
        assert_eq!(
            state
                .validation(&media)
                .as_ref()
                .err()
                .and_then(|error| audio_mix_ready_feedback(error)),
            None
        );
    }

    #[test]
    fn selected_tracks_do_not_override_invalid_output_and_feedback_clears_when_corrected() {
        let mut media = media();
        let source_path = std::env::temp_dir().join(format!(
            "apollo-audio-mix-ready-source-{}.mkv",
            std::process::id()
        ));
        let corrected_stem = format!("apollo-audio-mix-ready-corrected-{}", std::process::id());
        let _ = std::fs::remove_file(&source_path);
        std::fs::write(&source_path, b"input").expect("fixture input should be created");
        media.path = source_path.clone();
        let mut state = AudioMixSelectionState::new(&media);
        state.set_container(&media, OutputContainer::Mkv);
        state.set_custom_output_path(media.path.clone());

        let error = state.validation(&media).unwrap_err();
        assert!(matches!(
            &error,
            AudioMixValidationError::Output(MediaOutputError::SameInputAndOutput(_))
        ));
        assert!(!state.can_mix(&media));
        assert_eq!(
            audio_mix_ready_feedback(&error),
            Some("Output must be different from the source file.")
        );

        state.set_output_filename_stem(OsString::from(corrected_stem));

        assert!(state.validation(&media).is_ok());
        assert!(state.can_mix(&media));

        let corrected_output = state.output_path();
        std::fs::write(&corrected_output, b"occupied").expect("fixture output should be created");
        let collision = state.validation(&media).unwrap_err();
        assert!(matches!(
            &collision,
            AudioMixValidationError::Output(MediaOutputError::OutputAlreadyExists(_))
        ));
        assert!(!state.can_mix(&media));
        assert_eq!(
            audio_mix_ready_feedback(&collision),
            Some("Choose a different output filename or destination.")
        );

        std::fs::remove_file(corrected_output).expect("fixture output should be removable");
        assert!(state.can_mix(&media));

        std::fs::remove_file(source_path).expect("fixture input should be removable");
    }

    #[test]
    fn initial_container_is_mp4() {
        let state = AudioMixSelectionState::new(&media());

        assert_eq!(state.container(), OutputContainer::Mp4);
    }

    #[test]
    fn mkv_selection_updates_generated_default_extension() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);

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
        let mut state = AudioMixSelectionState::new(&media);
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
        let state = AudioMixSelectionState::new(&media);

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-mixed.mp4")
        );
        assert!(!state.output_is_customized());
    }

    #[test]
    fn custom_output_stem_is_preserved_across_container_changes() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);
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
        let mut state = AudioMixSelectionState::new(&media);

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
    fn empty_or_unusable_filename_stem_disables_mix_and_invalidates_request_output() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);

        for stem in ["", "   ", "../escape", "bad\\name"] {
            state.set_output_filename_stem(OsString::from(stem));
            assert!(!state.can_mix(&media), "{stem:?} should not be runnable");
            assert!(state.request(&media).output.as_os_str().is_empty());
            let error = state.validation(&media).unwrap_err();
            assert_eq!(
                audio_mix_ready_feedback(&error),
                Some("Enter a usable output filename.")
            );
        }
    }

    #[test]
    fn windows_reserved_device_stem_disables_mix() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);

        state.set_output_filename_stem(OsString::from("cOn"));

        assert!(!state.can_mix(&media));
        assert!(state.request(&media).output.as_os_str().is_empty());
    }

    #[test]
    fn missing_or_blank_title_falls_back_without_stream_numbers() {
        let mut media = media();
        media.audio_streams[0].title = Some("   ".to_owned());
        let rows = track_presentations(&media);

        assert_eq!(rows[0].title, UNTITLED_AUDIO);
        assert_eq!(rows[2].title, UNTITLED_AUDIO);
        assert!(!rows[0].title.contains('1'));
        assert!(!rows[2].title.contains('9'));
    }

    #[test]
    fn unique_titles_do_not_add_codec_or_language_metadata() {
        let rows = track_presentations(&media());

        assert_eq!(rows[0].metadata.as_deref(), Some("Stereo"));
        assert_eq!(rows[1].metadata.as_deref(), Some("Mono"));
        assert_eq!(rows[2].metadata.as_deref(), Some("Stereo"));
    }

    #[test]
    fn duplicate_titles_distinguished_by_channel_do_not_add_codec_or_language() {
        let mut media = media();
        media.audio_streams[0].title = Some("Commentary".to_owned());
        media.audio_streams[1].title = Some("Commentary".to_owned());
        let rows = track_presentations(&media);

        assert_eq!(rows[0].metadata.as_deref(), Some("Stereo"));
        assert_eq!(rows[1].metadata.as_deref(), Some("Mono"));
        assert_eq!(rows[2].metadata.as_deref(), Some("Stereo"));
    }

    #[test]
    fn duplicate_title_and_channel_use_language_when_it_distinguishes() {
        let mut media = media();
        media.audio_streams[0].title = Some("Commentary".to_owned());
        media.audio_streams[1].title = Some("Commentary".to_owned());
        media.audio_streams[1].channel_layout = Some("stereo".to_owned());
        media.audio_streams[1].channels = Some(2);
        media.audio_streams[1].language = Some("jpn".to_owned());
        let rows = track_presentations(&media);

        assert_eq!(rows[0].metadata.as_deref(), Some("Stereo · eng"));
        assert_eq!(rows[1].metadata.as_deref(), Some("Stereo · jpn"));
    }

    #[test]
    fn duplicate_title_channel_and_language_use_codec_when_it_distinguishes() {
        let mut media = media();
        media.audio_streams[0].title = Some("Commentary".to_owned());
        media.audio_streams[1].title = Some("Commentary".to_owned());
        media.audio_streams[1].channel_layout = Some("stereo".to_owned());
        media.audio_streams[1].channels = Some(2);
        media.audio_streams[1].language = Some("eng".to_owned());
        media.audio_streams[1].codec = Some("opus".to_owned());
        let rows = track_presentations(&media);

        assert_eq!(rows[0].metadata.as_deref(), Some("Stereo · aac"));
        assert_eq!(rows[1].metadata.as_deref(), Some("Stereo · opus"));
    }

    #[test]
    fn identical_language_and_codec_are_not_redundantly_appended() {
        let mut media = media();
        media.audio_streams[0].title = Some("Commentary".to_owned());
        media.audio_streams[1].title = Some("Commentary".to_owned());
        media.audio_streams[1].channel_layout = Some("stereo".to_owned());
        media.audio_streams[1].channels = Some(2);
        media.audio_streams[1].language = Some("eng".to_owned());
        media.audio_streams[1].codec = Some("aac".to_owned());
        let rows = track_presentations(&media);

        assert_eq!(rows[0].metadata.as_deref(), Some("Stereo"));
        assert_eq!(rows[1].metadata.as_deref(), Some("Stereo"));
    }

    #[test]
    fn missing_disambiguation_metadata_remains_safe_and_does_not_use_stream_numbers() {
        let mut media = media();
        for stream in &mut media.audio_streams[..2] {
            stream.title = None;
            stream.channel_layout = None;
            stream.channels = None;
            stream.language = None;
            stream.codec = None;
        }
        let rows = track_presentations(&media);

        assert_eq!(rows[0].title, UNTITLED_AUDIO);
        assert_eq!(rows[1].title, UNTITLED_AUDIO);
        assert_eq!(rows[0].metadata, None);
        assert_eq!(rows[1].metadata, None);
        assert!(!rows[0].title.contains('1'));
        assert!(!rows[1].title.contains('4'));
    }

    #[test]
    fn request_preserves_source_order_for_selected_absolute_indexes() {
        let mut media = media();
        media.audio_streams.swap(0, 2);
        let mut state = AudioMixSelectionState::new(&media);
        state.toggle_stream(&media, 4);

        let request = state.request(&media);

        assert_eq!(request.selected_streams, vec![9, 1]);
        assert_eq!(request.input, media.path);
        assert_eq!(
            request.output,
            PathBuf::from("captures/recording-mixed.mp4")
        );
    }

    #[test]
    fn source_metadata_uses_dimensions_and_duration() {
        assert_eq!(
            source_metadata(&media()).as_deref(),
            Some("3840×2160 · 01:42:16")
        );
    }

    #[test]
    fn minimum_height_uses_outer_task_scroll_when_fixed_audio_mix_flow_cannot_fit() {
        let context = egui::Context::default();
        super::fonts::install(&context);

        let output = context.run_ui(Default::default(), |ui| {
            let media = media();
            let usable_height =
                layout::MINIMUM_CONTENT_SIZE.y - f32::from(layout::CONTENT_PADDING) * 2.0;
            let fixed_height = audio_mix_fixed_flow_height(ui, &media, false, false);
            let plan = layout::task_surface_plan(
                usable_height,
                fixed_height,
                12.0 * theme::SELECTABLE_ROW_MIN_HEIGHT,
            );

            assert_eq!(usable_height, 204.0);
            assert!(fixed_height > usable_height);
            assert_eq!(plan.overflow, layout::TaskOverflow::Scroll);
            assert!(plan.list.max_height >= theme::SELECTABLE_ROW_MIN_HEIGHT);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn reference_height_keeps_long_track_list_nested_and_task_surface_in_flow() {
        let context = egui::Context::default();
        super::fonts::install(&context);

        let output = context.run_ui(Default::default(), |ui| {
            let media = media();
            let usable_height =
                layout::REFERENCE_CONTENT_SIZE.y - f32::from(layout::CONTENT_PADDING) * 2.0;
            let fixed_height = audio_mix_fixed_flow_height(ui, &media, false, false);
            let plan = layout::task_surface_plan(
                usable_height,
                fixed_height,
                12.0 * theme::SELECTABLE_ROW_MIN_HEIGHT,
            );

            assert_eq!(usable_height, 323.0);
            assert_eq!(plan.overflow, layout::TaskOverflow::Flow);
            assert_eq!(plan.list.behavior, layout::ListBehavior::Scroll);
            assert_eq!(
                plan.list.max_height,
                (usable_height - fixed_height).max(0.0)
            );
            assert!(plan.list.max_height >= theme::SELECTABLE_ROW_MIN_HEIGHT);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn taller_height_exposes_more_audio_tracks_before_scrolling() {
        let context = egui::Context::default();
        super::fonts::install(&context);

        let output = context.run_ui(Default::default(), |ui| {
            let media = media();
            let fixed_height = audio_mix_fixed_flow_height(ui, &media, false, false);
            let content_height = 20.0 * theme::SELECTABLE_ROW_MIN_HEIGHT;
            let reference = layout::task_surface_plan(323.0, fixed_height, content_height);
            let tall = layout::task_surface_plan(700.0, fixed_height, content_height);

            assert_eq!(reference.list.behavior, layout::ListBehavior::Scroll);
            assert_eq!(tall.list.behavior, layout::ListBehavior::Scroll);
            assert!(tall.list.max_height > reference.list.max_height);
            assert_eq!(tall.overflow, layout::TaskOverflow::Flow);
        });
        output.drop_without_applying_deltas();
    }
}

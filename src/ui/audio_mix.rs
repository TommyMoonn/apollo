use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use egui::{DragValue, Response, RichText, Sides, Ui, WidgetInfo, WidgetType};

use crate::{
    audio_mix::{
        model::{
            AudioMixRequest, LoudnessNormalization, LoudnessTargets, OutputContainer,
            SelectedAudioTrack, TrackGain,
        },
        validation::{AudioMixValidationError, validate_audio_mix_request},
    },
    media::{
        model::{AudioStream, MediaFile},
        output::MediaOutputError,
    },
};

use super::{
    ADD_TO_QUEUE_LABEL, CANCEL_QUEUE_EDIT_LABEL, OperationSurfaceMode, SAVE_QUEUE_CHANGES_LABEL,
    fonts, layout, output_name::OutputNameState, theme, widgets,
};

const UNTITLED_AUDIO: &str = "Untitled audio";
const AUDIO_TRACKS_LABEL: &str = "Audio tracks";
const OUTPUT_LABEL: &str = "Output";
const SELECT_ALL_LABEL: &str = "Select all";
const CLEAR_LABEL: &str = "Clear";
const RESET_GAINS_LABEL: &str = "Reset gains";
const NORMALIZE_LABEL: &str = "Normalize output loudness";
const LOUDNESS_NORMALIZATION_LABEL: &str = "Loudness normalization";
const INTEGRATED_LOUDNESS_LABEL: &str = "Integrated loudness";
const MAXIMUM_TRUE_PEAK_LABEL: &str = "Maximum true peak";
const LOUDNESS_RANGE_LABEL: &str = "Loudness range";
const RESET_NORMALIZATION_LABEL: &str = "Reset normalization targets";
const BROWSE_LABEL: &str = "Browse";
const MIX_LABEL: &str = "Mix selected tracks";
const CONTAINER_OPTIONS: [&str; 2] = ["MP4", "MKV"];

/// Pre-execution Audio Mix state. Process lifecycle remains owned by `AudioMixExecutor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMixSelectionState {
    selected_streams: BTreeSet<u32>,
    track_gains: BTreeMap<u32, TrackGain>,
    normalization_enabled: bool,
    normalization_targets: LoudnessTargets,
    container: OutputContainer,
    output_name: OutputNameState,
    ready_validation: AudioMixReadyValidationCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioMixReadyValidationCache {
    dirty: bool,
    result: Result<(), AudioMixValidationError>,
    #[cfg(test)]
    rebuild_count: usize,
}

impl Default for AudioMixReadyValidationCache {
    fn default() -> Self {
        Self {
            dirty: true,
            result: Ok(()),
            #[cfg(test)]
            rebuild_count: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioMixSelectionAction {
    #[default]
    None,
    BrowseOutput,
    QueueRequested,
    MixRequested,
    SaveQueueChanges,
    CancelQueueEdit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMixPresentation {
    tracks: Vec<TrackPresentation>,
}

impl AudioMixPresentation {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            tracks: track_presentations(media),
        }
    }
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
        let mut state = Self {
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
        };
        let _ = state.validation(media);
        state
    }

    pub fn from_request(media: &MediaFile, request: &AudioMixRequest) -> Self {
        let mut state = Self::new(media);
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
        state.invalidate_ready_validation();
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

    fn validation(&mut self, media: &MediaFile) -> &Result<(), AudioMixValidationError> {
        if self.ready_validation.dirty {
            self.ready_validation.result = validate_audio_mix_request(&self.request(media), media);
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
    fn ready_validation_rebuild_count(&self) -> usize {
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

    /// Build the Phase 24 request without starting work. Stream order follows probe/source order.
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

/// Render the complete pre-execution Audio Mix configuration surface.
///
/// The caller owns source acquisition and execution orchestration. This surface only mutates
/// selection, container, and output-path state and emits user intent for those responsibilities.
pub fn show(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &AudioMixPresentation,
    state: &mut AudioMixSelectionState,
    mode: impl Into<OperationSurfaceMode>,
) -> AudioMixSelectionAction {
    let mode = mode.into();
    let action = Cell::new(AudioMixSelectionAction::None);
    let (can_mix, feedback) = {
        let validation = state.validation(media);
        (
            validation.is_ok(),
            validation.as_ref().err().and_then(audio_mix_ready_feedback),
        )
    };

    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            show_track_header(ui, media, state);
            ui.add_space(theme::SPACING.compact);
            show_track_list(ui, media, state, &presentation.tracks);
            ui.add_space(theme::INLINE_ACTION_GAP);
            show_track_footer(ui, state);
            ui.add_space(theme::SPACING.related);

            show_loudness_header(ui, state);
            ui.add_space(theme::SPACING.compact);
            widgets::separator(ui);
            let normalization_enabled = state.normalization_enabled();
            let _ = widgets::conditional_fade(
                ui,
                "audio-mix-normalization-targets",
                normalization_enabled,
                |ui| {
                    ui.add_space(theme::SPACING.compact);
                    show_normalization_targets(ui, state);
                },
            );
            ui.add_space(theme::SPACING.section);

            let _ = widgets::section_label(ui, OUTPUT_LABEL);
            ui.add_space(theme::SPACING.compact);
            let output_action = show_output_row(ui, media, state);
            if output_action != AudioMixSelectionAction::None {
                action.set(output_action);
            }

            if let Some(feedback) = feedback {
                ui.add_space(theme::SPACING.tight);
                let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
            }
        },
        |ui| match mode {
            OperationSurfaceMode::Immediate { start_allowed } => {
                if widgets::primary_button_enabled(ui, MIX_LABEL, can_mix && start_allowed)
                    .clicked()
                {
                    action.set(AudioMixSelectionAction::MixRequested);
                }
                if ui
                    .add_enabled_ui(can_mix, |ui| {
                        widgets::secondary_button(ui, ADD_TO_QUEUE_LABEL)
                    })
                    .inner
                    .clicked()
                {
                    action.set(AudioMixSelectionAction::QueueRequested);
                }
            }
            OperationSurfaceMode::QueueEdit => {
                if widgets::primary_button_enabled(ui, SAVE_QUEUE_CHANGES_LABEL, can_mix).clicked()
                {
                    action.set(AudioMixSelectionAction::SaveQueueChanges);
                }
                if widgets::secondary_button(ui, CANCEL_QUEUE_EDIT_LABEL).clicked() {
                    action.set(AudioMixSelectionAction::CancelQueueEdit);
                }
            }
        },
    );

    action.get()
}

fn show_track_list(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut AudioMixSelectionState,
    tracks: &[TrackPresentation],
) {
    widgets::separator(ui);

    if tracks.is_empty() {
        widgets::separator(ui);
        return;
    }

    for track in tracks {
        let gain = state.track_gain(track.stream_index).unwrap_or_default();
        let response = widgets::selectable_row_with_accessory(
            ui,
            &track.title,
            track.metadata.as_deref(),
            state.is_selected(track.stream_index),
            widgets::SelectionIndicator::Checkbox,
            theme::TRACK_GAIN_CONTROL_WIDTH,
            |ui| track_gain_control(ui, &track.title, gain),
        );

        if response.selection.clicked() {
            state.toggle_stream(media, track.stream_index);
        }
        if response.accessory.0.changed() {
            state.set_track_gain(media, track.stream_index, response.accessory.1);
        }
    }
}

fn track_gain_control(ui: &mut Ui, title: &str, gain: TrackGain) -> (Response, TrackGain) {
    let mut tenths_db = gain.tenths_db();
    let response = ui.add_sized(
        [
            theme::TRACK_GAIN_CONTROL_WIDTH,
            theme::COMPACT_CONTROL_HEIGHT,
        ],
        DragValue::new(&mut tenths_db)
            .range(TrackGain::MIN.tenths_db()..=TrackGain::MAX.tenths_db())
            .speed(5.0)
            .custom_formatter(|value, _| format!("{:.1}", value / 10.0))
            .custom_parser(|text| {
                text.trim()
                    .strip_suffix("dB")
                    .unwrap_or(text)
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .map(|value| value * 10.0)
            })
            .suffix(" dB"),
    );
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::DragValue,
            response.enabled(),
            format!("{title} gain"),
        )
    });

    if response.changed() {
        tenths_db = stepped_track_gain(tenths_db).tenths_db();
    }

    (response, TrackGain::from_tenths_db(tenths_db))
}

fn stepped_track_gain(tenths_db: i16) -> TrackGain {
    let tenths_db = ((f64::from(tenths_db) / 5.0).round() as i16 * 5)
        .clamp(TrackGain::MIN.tenths_db(), TrackGain::MAX.tenths_db());
    TrackGain::from_tenths_db(tenths_db)
}

fn show_track_header(ui: &mut Ui, media: &MediaFile, state: &mut AudioMixSelectionState) {
    Sides::new().height(theme::COMPACT_CONTROL_HEIGHT).show(
        ui,
        |ui| widgets::section_label(ui, AUDIO_TRACKS_LABEL),
        |ui| {
            widgets::trailing_control(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::INLINE_ACTION_GAP;
                    if widgets::plain_button(ui, SELECT_ALL_LABEL).clicked() {
                        state.select_all(media);
                    }
                    if widgets::plain_button(ui, CLEAR_LABEL).clicked() {
                        state.clear();
                    }
                });
            });
        },
    );
}

fn show_track_footer(ui: &mut Ui, state: &mut AudioMixSelectionState) {
    let selected_count = state.selected_count();

    Sides::new().height(theme::COMPACT_CONTROL_HEIGHT).show(
        ui,
        |ui| {
            ui.label(
                RichText::new(format!("{selected_count} selected"))
                    .text_style(fonts::technical_text_style())
                    .color(theme::SECONDARY_FAINT),
            );
        },
        |ui| {
            widgets::trailing_control(ui, |ui| {
                if widgets::plain_button(ui, RESET_GAINS_LABEL).clicked() {
                    state.reset_track_gains();
                }
            });
        },
    );
}

fn show_loudness_header(ui: &mut Ui, state: &mut AudioMixSelectionState) {
    let mut enabled = state.normalization_enabled();
    let (_, response) = Sides::new()
        .height(theme::COMPACT_CONTROL_HEIGHT)
        .spacing(theme::SPACING.related)
        .shrink_left()
        .truncate()
        .show(
            ui,
            |ui| widgets::section_label(ui, LOUDNESS_NORMALIZATION_LABEL),
            |ui| {
                widgets::trailing_control(ui, |ui| {
                    widgets::switch_control(ui, NORMALIZE_LABEL, &mut enabled)
                })
            },
        );

    if response.changed() {
        state.set_normalization_enabled(enabled);
    }
}

fn show_normalization_targets(ui: &mut Ui, state: &mut AudioMixSelectionState) {
    let targets = state.normalization_targets();
    let (integrated_response, integrated) = normalization_target_control(
        ui,
        INTEGRATED_LOUDNESS_LABEL,
        targets.integrated_loudness_tenths(),
        LoudnessTargets::MIN_INTEGRATED_LOUDNESS_TENTHS,
        LoudnessTargets::MAX_INTEGRATED_LOUDNESS_TENTHS,
        5,
        " LUFS",
    );
    ui.add_space(theme::SPACING.tight);
    let (true_peak_response, true_peak) = normalization_target_control(
        ui,
        MAXIMUM_TRUE_PEAK_LABEL,
        targets.maximum_true_peak_tenths(),
        LoudnessTargets::MIN_MAXIMUM_TRUE_PEAK_TENTHS,
        LoudnessTargets::MAX_MAXIMUM_TRUE_PEAK_TENTHS,
        1,
        " dBTP",
    );
    ui.add_space(theme::SPACING.tight);
    let (range_response, range) = normalization_target_control(
        ui,
        LOUDNESS_RANGE_LABEL,
        targets.loudness_range_tenths(),
        LoudnessTargets::MIN_LOUDNESS_RANGE_TENTHS,
        LoudnessTargets::MAX_LOUDNESS_RANGE_TENTHS,
        5,
        " LU",
    );

    if integrated_response.changed() || true_peak_response.changed() || range_response.changed() {
        let _ = state
            .set_normalization_targets(LoudnessTargets::from_tenths(integrated, true_peak, range));
    }

    ui.add_space(theme::SPACING.tight);
    if widgets::plain_button(ui, RESET_NORMALIZATION_LABEL).clicked() {
        state.reset_normalization_targets();
    }
}

#[allow(clippy::too_many_arguments)]
fn normalization_target_control(
    ui: &mut Ui,
    label: &str,
    mut tenths: i16,
    minimum: i16,
    maximum: i16,
    step: i16,
    suffix: &'static str,
) -> (Response, i16) {
    let response = widgets::labeled_control(ui, label, |ui| {
        ui.add_sized(
            [
                theme::TRACK_GAIN_CONTROL_WIDTH,
                theme::COMPACT_CONTROL_HEIGHT,
            ],
            DragValue::new(&mut tenths)
                .range(minimum..=maximum)
                .speed(f64::from(step))
                .custom_formatter(|value, _| format!("{:.1}", value / 10.0))
                .custom_parser(move |text| {
                    text.trim()
                        .strip_suffix(suffix.trim())
                        .unwrap_or(text)
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .map(|value| value * 10.0)
                })
                .suffix(suffix),
        )
    });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::DragValue, response.enabled(), label));

    if response.changed() {
        tenths = stepped_tenths(tenths, step, minimum, maximum);
    }

    (response, tenths)
}

fn stepped_tenths(value: i16, step: i16, minimum: i16, maximum: i16) -> i16 {
    ((f64::from(value) / f64::from(step)).round() as i16 * step).clamp(minimum, maximum)
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
        AudioMixValidationError::UnsupportedChannelLayout { .. } => {
            Some("Audio Mix currently supports mono and stereo tracks only.")
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

    let row = widgets::operation_output_row(
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
        media::model::{AudioStream, MediaFile, VideoStream},
        ui::{fonts, theme},
    };

    use super::{
        AudioMixPresentation, AudioMixSelectionAction, AudioMixSelectionState, UNTITLED_AUDIO,
        audio_mix_ready_feedback, stepped_tenths, stepped_track_gain, track_presentations,
    };
    use crate::audio_mix::model::{
        LoudnessNormalization, LoudnessTargets, OutputContainer, SelectedAudioTrack, TrackGain,
    };
    use crate::audio_mix::validation::AudioMixValidationError;

    fn media() -> MediaFile {
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
    fn presentation_changes_only_when_probe_data_changes() {
        let original = media();
        let original_presentation = AudioMixPresentation::new(&original);

        let mut replaced = original.clone();
        replaced.audio_streams[0].title = Some("Game Audio".to_owned());
        let replaced_presentation = AudioMixPresentation::new(&replaced);

        assert_ne!(original_presentation, replaced_presentation);
    }

    #[test]
    fn selection_changes_do_not_rebuild_or_mutate_track_presentation() {
        let media = media();
        let presentation = AudioMixPresentation::new(&media);
        let expected = presentation.clone();
        let mut state = AudioMixSelectionState::new(&media);

        assert!(state.toggle_stream(&media, 4));

        assert_eq!(presentation, expected);
        assert!(!state.is_selected(4));
        assert!(
            presentation
                .tracks
                .iter()
                .any(|track| track.stream_index == 4 && track.title == "Microphone")
        );
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
        let mut state = AudioMixSelectionState::new(&media);

        assert!(state.can_mix(&media));
    }

    #[test]
    fn unchanged_ready_repaints_reuse_cached_validation() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let media = media();
        let presentation = AudioMixPresentation::new(&media);
        let mut state = AudioMixSelectionState::new(&media);
        assert_eq!(state.ready_validation_rebuild_count(), 1);

        for _ in 0..2 {
            let output = context.run_ui(Default::default(), |ui| {
                assert_eq!(
                    super::show(ui, &media, &presentation, &mut state, true),
                    AudioMixSelectionAction::None
                );
            });
            output.drop_without_applying_deltas();
        }

        assert_eq!(state.ready_validation_rebuild_count(), 1);
    }

    #[test]
    fn relevant_state_changes_recompute_ready_validation_once() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);
        let mut expected_rebuilds = 1;

        state.clear();
        assert!(!state.can_mix(&media));
        expected_rebuilds += 1;
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);

        state.select_all(&media);
        assert!(state.can_mix(&media));
        expected_rebuilds += 1;
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);

        assert!(state.set_track_gain(&media, 4, TrackGain::from_tenths_db(-55)));
        assert!(state.can_mix(&media));
        expected_rebuilds += 1;
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);

        state.set_normalization_enabled(true);
        assert!(state.can_mix(&media));
        expected_rebuilds += 1;
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);

        assert!(state.set_normalization_targets(LoudnessTargets::from_tenths(-220, -8, 145)));
        assert!(state.can_mix(&media));
        expected_rebuilds += 1;
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);

        state.set_container(&media, OutputContainer::Mkv);
        assert!(state.can_mix(&media));
        expected_rebuilds += 1;
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);

        state.set_output_filename_stem(OsString::from(""));
        assert!(!state.can_mix(&media));
        expected_rebuilds += 1;
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);

        state.set_custom_output_path(PathBuf::from("exports/final-mix.mkv"));
        assert!(state.can_mix(&media));
        expected_rebuilds += 1;
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);

        assert!(state.can_mix(&media));
        assert_eq!(state.ready_validation_rebuild_count(), expected_rebuilds);
    }

    #[test]
    fn replacement_source_builds_source_bound_ready_validation() {
        let original = media();
        let mut original_state = AudioMixSelectionState::new(&original);
        assert!(original_state.can_mix(&original));

        let mut replacement = original.clone();
        replacement.path = PathBuf::from("captures/replacement.mkv");
        replacement.video_streams.clear();
        let mut replacement_state = AudioMixSelectionState::new(&replacement);

        assert!(!replacement_state.can_mix(&replacement));
        assert_eq!(replacement_state.ready_validation_rebuild_count(), 1);
        assert_eq!(
            replacement_state.validation(&replacement).as_ref(),
            Err(&AudioMixValidationError::MissingVideoStream)
        );
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
    fn ready_validation_ignores_destination_filesystem_state() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);
        state.set_container(&media, OutputContainer::Mkv);

        state.set_custom_output_path(media.path.clone());
        assert!(state.validation(&media).is_ok());
        assert!(state.can_mix(&media));

        state.set_custom_output_path(PathBuf::from("missing-parent/output.mkv"));
        assert!(state.validation(&media).is_ok());
        assert!(state.can_mix(&media));
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
            let error = state.validation(&media).as_ref().unwrap_err();
            assert_eq!(
                audio_mix_ready_feedback(error),
                Some("Enter a usable output filename.")
            );
        }
    }

    #[test]
    fn unsupported_channel_layout_has_ready_state_feedback() {
        let error = AudioMixValidationError::UnsupportedChannelLayout {
            stream_index: 8,
            channels: Some(6),
            channel_layout: Some("5.1".to_owned()),
        };

        assert_eq!(
            audio_mix_ready_feedback(&error),
            Some("Audio Mix currently supports mono and stereo tracks only.")
        );
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
        let mut state = AudioMixSelectionState::new(&media);
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
        let mut state = AudioMixSelectionState::new(&media);

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
        let mut state = AudioMixSelectionState::new(&media);
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
        let mut state = AudioMixSelectionState::new(&media);
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
    fn gain_ui_step_rounds_to_half_decibels_within_domain_bounds() {
        assert_eq!(stepped_track_gain(-63), TrackGain::from_tenths_db(-65));
        assert_eq!(stepped_track_gain(-62), TrackGain::from_tenths_db(-60));
        assert_eq!(stepped_track_gain(117), TrackGain::from_tenths_db(115));
        assert_eq!(stepped_track_gain(123), TrackGain::MAX);
        assert_eq!(stepped_track_gain(-603), TrackGain::MIN);
    }

    #[test]
    fn gain_state_rejects_unknown_streams_and_out_of_range_values() {
        let media = media();
        let mut state = AudioMixSelectionState::new(&media);

        assert!(!state.set_track_gain(&media, 99, TrackGain::ZERO));
        assert!(!state.set_track_gain(&media, 1, TrackGain::from_tenths_db(121)));
        assert_eq!(state.track_gain(1), Some(TrackGain::ZERO));
    }

    #[test]
    fn new_session_starts_with_normalization_off_and_default_targets() {
        let state = AudioMixSelectionState::new(&media());

        assert!(!state.normalization_enabled());
        assert_eq!(state.normalization_targets(), LoudnessTargets::default());
    }

    #[test]
    fn normalization_toggle_preserves_edited_targets() {
        let mut state = AudioMixSelectionState::new(&media());
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
        let mut state = AudioMixSelectionState::new(&media);
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
        let mut state = AudioMixSelectionState::new(&media);
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
        let mut state = AudioMixSelectionState::new(&media());

        assert!(!state.set_normalization_targets(LoudnessTargets::from_tenths(-701, -15, 110)));
        assert!(!state.set_normalization_targets(LoudnessTargets::from_tenths(-160, 1, 110)));
        assert!(!state.set_normalization_targets(LoudnessTargets::from_tenths(-160, -15, 501)));
        assert_eq!(state.normalization_targets(), LoudnessTargets::default());
    }

    #[test]
    fn normalization_ui_steps_and_clamps_each_target_range() {
        assert_eq!(stepped_tenths(-703, 5, -700, -50), -700);
        assert_eq!(stepped_tenths(-52, 5, -700, -50), -50);
        assert_eq!(stepped_tenths(-16, 1, -90, 0), -16);
        assert_eq!(stepped_tenths(1, 1, -90, 0), 0);
        assert_eq!(stepped_tenths(12, 5, 10, 500), 10);
        assert_eq!(stepped_tenths(498, 5, 10, 500), 500);
    }
}

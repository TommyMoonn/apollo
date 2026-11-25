use crate::audio_mix::draft::AudioMixDraft;

use std::{cell::Cell, collections::HashMap, ffi::OsString};

use egui::{DragValue, Response, RichText, Sides, Ui, WidgetInfo, WidgetType};

use crate::{
    audio_mix::{
        model::{LoudnessTargets, OutputContainer, TrackGain},
        validation::AudioMixValidationError,
    },
    media::{
        model::{AudioStream, MediaFile},
        output::MediaOutputError,
    },
};

use super::{
    OperationSurfaceMode, fonts, layout, media_error, show_queued_submission_actions, theme,
    widgets,
};

const UNTITLED_AUDIO: &str = "Untitled audio";
const AUDIO_TRACKS_LABEL: &str = "Audio tracks";
const OUTPUT_LABEL: &str = "Output";
const OUTPUT_CONTAINER_LABEL: &str = "Output container";
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
const CONTAINER_OPTIONS: [&str; 2] = ["MP4", "MKV"];

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

/// Render the complete pre-execution Audio Mix configuration surface.
///
/// The caller owns source acquisition and execution orchestration. This surface only mutates
/// selection, container, and output-path state and emits user intent for those responsibilities.
pub fn show<'a>(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &AudioMixPresentation,
    state: &mut AudioMixDraft,
    mode: impl Into<OperationSurfaceMode<'a>>,
) -> AudioMixSelectionAction {
    let mode = mode.into();
    let shared_defaults = matches!(mode, OperationSurfaceMode::BatchDefaults { .. });
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

            if !shared_defaults {
                show_track_header(ui, media, state);
                ui.add_space(theme::SPACING.compact);
                show_track_list(ui, media, state, &presentation.tracks);
                ui.add_space(theme::INLINE_ACTION_GAP);
                show_track_footer(ui, state);
                ui.add_space(theme::SPACING.related);
            }

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

            if shared_defaults {
                let _ = widgets::section_label(ui, OUTPUT_CONTAINER_LABEL);
                ui.add_space(theme::SPACING.compact);
                show_container_selector(ui, media, state);
            } else {
                let _ = widgets::section_label(ui, OUTPUT_LABEL);
                ui.add_space(theme::SPACING.compact);
                let output_action = show_output_row(ui, media, state);
                if output_action != AudioMixSelectionAction::None {
                    action.set(output_action);
                }
            }

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
                match show_queued_submission_actions(ui, start_allowed, can_mix, queue_confirmation)
                {
                    Some(crate::app::queue::QueueSubmission::ProcessNow) => {
                        action.set(AudioMixSelectionAction::MixRequested)
                    }
                    Some(crate::app::queue::QueueSubmission::AddToQueue) => {
                        action.set(AudioMixSelectionAction::QueueRequested)
                    }
                    None => {}
                }
            }
            edit_mode => {
                let (save_label, cancel_label) = edit_mode
                    .edit_labels()
                    .expect("non-immediate operation mode has edit labels");
                let save_allowed = match edit_mode {
                    OperationSurfaceMode::BatchDefaults { save_allowed } => save_allowed,
                    _ => can_mix,
                };
                if widgets::primary_button_enabled(ui, save_label, save_allowed).clicked() {
                    action.set(AudioMixSelectionAction::SaveQueueChanges);
                }
                if widgets::secondary_button(ui, cancel_label).clicked() {
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
    state: &mut AudioMixDraft,
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

fn show_track_header(ui: &mut Ui, media: &MediaFile, state: &mut AudioMixDraft) {
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

fn show_track_footer(ui: &mut Ui, state: &mut AudioMixDraft) {
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

fn show_loudness_header(ui: &mut Ui, state: &mut AudioMixDraft) {
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

fn show_normalization_targets(ui: &mut Ui, state: &mut AudioMixDraft) {
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
    state: &mut AudioMixDraft,
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
        |ui| select_container(ui, state, &mut requested_container),
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

fn show_container_selector(ui: &mut Ui, media: &MediaFile, state: &mut AudioMixDraft) {
    let mut requested = None;
    select_container(ui, state, &mut requested);
    if let Some(container) = requested {
        state.set_container(media, container);
    }
}

fn select_container(ui: &mut Ui, state: &AudioMixDraft, requested: &mut Option<OutputContainer>) {
    widgets::select_menu(
        ui,
        container_label(state.container()),
        &CONTAINER_OPTIONS,
        |ui| {
            for (label, container) in [("MP4", OutputContainer::Mp4), ("MKV", OutputContainer::Mkv)]
            {
                if widgets::select_menu_option(ui, label, state.container() == container).clicked()
                {
                    *requested = Some(container);
                    ui.close();
                }
            }
        },
    );
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

fn container_label(container: OutputContainer) -> &'static str {
    match container {
        OutputContainer::Mp4 => "MP4",
        OutputContainer::Mkv => "MKV",
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use crate::ui::{fonts, theme};

    use super::{
        AudioMixDraft, AudioMixPresentation, AudioMixSelectionAction, UNTITLED_AUDIO,
        audio_mix_ready_feedback, stepped_tenths, stepped_track_gain, track_presentations,
    };
    use crate::audio_mix::model::TrackGain;
    use crate::audio_mix::validation::AudioMixValidationError;

    use crate::audio_mix::draft::tests::media;

    #[test]
    fn unchanged_ready_repaints_reuse_cached_validation() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let media = media();
        let presentation = AudioMixPresentation::new(&media);
        let mut state = AudioMixDraft::new(&media);
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
    fn gain_ui_step_rounds_to_half_decibels_within_domain_bounds() {
        assert_eq!(stepped_track_gain(-63), TrackGain::from_tenths_db(-65));
        assert_eq!(stepped_track_gain(-62), TrackGain::from_tenths_db(-60));
        assert_eq!(stepped_track_gain(117), TrackGain::from_tenths_db(115));
        assert_eq!(stepped_track_gain(123), TrackGain::MAX);
        assert_eq!(stepped_track_gain(-603), TrackGain::MIN);
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
    #[test]
    fn empty_or_unusable_filename_stem_disables_mix_and_invalidates_request_output() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);

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
    fn clear_all_disables_primary_action() {
        let media = media();
        let mut state = AudioMixDraft::new(&media);

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
}

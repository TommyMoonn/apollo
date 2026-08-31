use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use egui::{Align, Label, Layout, RichText, Sides, TextStyle, Ui};

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

use super::{fonts, layout, output_name::OutputNameState, theme, widgets};

const CONTAINER_LABEL: &str = "Container";
const OUTPUT_LABEL: &str = "Output";
const BROWSE_LABEL: &str = "Browse";
const REMUX_LABEL: &str = "Remux";
const CONTAINER_OPTIONS: [&str; 2] = ["MP4", "MKV"];

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
    RemuxRequested,
}

impl RemuxSelectionState {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            container: MediaContainer::Matroska,
            output_name: OutputNameState::new(&media.path, default_output_stem(&media.path)),
        }
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
    state: &mut RemuxSelectionState,
) -> RemuxSelectionAction {
    let mut action = RemuxSelectionAction::None;
    let validation = state.validation(media);
    let feedback = validation
        .as_ref()
        .err()
        .and_then(|error| remux_ready_feedback(error, media));
    let plan = layout::task_surface_plan(
        ui.available_height(),
        remux_fixed_flow_height(ui, media, feedback.as_deref()),
        0.0,
        0.0,
    );

    layout::task_surface(ui, plan.overflow, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;

        show_source_header(ui, media);
        ui.add_space(theme::SPACING.section);

        let _ = widgets::section_label(ui, CONTAINER_LABEL);
        ui.add_space(theme::SPACING.compact);
        show_container_selector(ui, state);

        ui.add_space(theme::SPACING.focal);
        let _ = widgets::section_label(ui, OUTPUT_LABEL);
        ui.add_space(theme::SPACING.compact);
        let mut output_stem = state.output_filename_stem().to_string_lossy().into_owned();
        let row = widgets::output_filename_row(
            ui,
            &mut output_stem,
            state.container().extension(),
            BROWSE_LABEL,
        );
        if row.filename.changed() {
            state.set_output_filename_stem(OsString::from(output_stem));
        }
        if row.browse.clicked() {
            action = RemuxSelectionAction::BrowseOutput;
        }

        if let Some(feedback) = feedback.as_deref() {
            ui.add_space(theme::SPACING.tight);
            let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
        }

        ui.add_space(theme::SPACING.related);
        let response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::primary_button_enabled(ui, REMUX_LABEL, validation.is_ok())
            })
            .inner;
        if response.clicked() {
            action = RemuxSelectionAction::RemuxRequested;
        }
    });

    action
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

fn remux_ready_feedback(error: &RemuxValidationError, media: &MediaFile) -> Option<String> {
    match error {
        RemuxValidationError::NoStreams => {
            Some("This source has no media streams to remux.".to_owned())
        }
        RemuxValidationError::IncompatibleStream {
            container,
            stream_index,
            kind,
            codec,
        } => {
            let selected = container_label(*container);
            let other = match container {
                MediaContainer::Mp4 => MediaContainer::Matroska,
                MediaContainer::Matroska => MediaContainer::Mp4,
            };
            let stream = stream_description(*stream_index, *kind, codec.as_deref());
            if other.is_inventory_stream_copy_compatible(&media.streams) {
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

fn show_source_header(ui: &mut Ui, media: &MediaFile) {
    let metadata = inventory_summary(media);
    let metadata_gap = if metadata.is_some() {
        theme::SPACING.tight
    } else {
        0.0
    };
    let header_height = source_header_height(ui, metadata.is_some());

    Sides::new()
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
                        Label::new(
                            RichText::new(source_file_name(&media.path))
                                .text_style(TextStyle::Body),
                        )
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
            |_| {},
        );
}

fn remux_fixed_flow_height(ui: &Ui, media: &MediaFile, feedback: Option<&str>) -> f32 {
    let section_label_height = ui.text_style_height(&TextStyle::Small);
    let output_row_height = widgets::output_filename_row_plan(ui, None, BROWSE_LABEL).flow_height();
    let feedback_height = feedback.map_or(0.0, |feedback| {
        theme::SPACING.tight + widgets::inline_status_height(ui, feedback, ui.available_width())
    });

    source_header_height(ui, inventory_summary(media).is_some())
        + theme::SPACING.section
        + section_label_height
        + theme::SPACING.compact
        + theme::COMPACT_CONTROL_HEIGHT
        + theme::SPACING.focal
        + section_label_height
        + theme::SPACING.compact
        + output_row_height
        + feedback_height
        + theme::SPACING.related
        + theme::PRIMARY_CONTROL_HEIGHT
}

fn source_header_height(ui: &Ui, has_metadata: bool) -> f32 {
    let metadata_height = if has_metadata {
        theme::SPACING.tight + ui.text_style_height(&fonts::technical_text_style())
    } else {
        0.0
    };
    (ui.text_style_height(&TextStyle::Body) + metadata_height).max(theme::COMPACT_CONTROL_HEIGHT)
}

fn source_file_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn inventory_summary(media: &MediaFile) -> Option<String> {
    let mut counts = [0_usize; 6];
    for stream in &media.streams {
        counts[match stream.kind {
            StreamKind::Video => 0,
            StreamKind::Audio => 1,
            StreamKind::Subtitle => 2,
            StreamKind::Data => 3,
            StreamKind::Attachment => 4,
            StreamKind::Unknown => 5,
        }] += 1;
    }

    let mut parts = Vec::new();
    for (count, singular, plural) in [
        (counts[0], "video", "video"),
        (counts[1], "audio", "audio"),
        (counts[2], "subtitle", "subtitles"),
        (counts[3], "data", "data"),
        (counts[4], "attachment", "attachments"),
        (counts[5], "unknown stream", "unknown streams"),
    ] {
        if count > 0 {
            parts.push(format!(
                "{count} {}",
                if count == 1 { singular } else { plural }
            ));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
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
        ui::{fonts, layout, theme},
    };

    use super::{
        RemuxSelectionState, inventory_summary, remux_fixed_flow_height, remux_ready_feedback,
    };

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
        assert_eq!(
            remux_ready_feedback(
                &RemuxValidationError::IncompatibleStream {
                    container: MediaContainer::Mp4,
                    stream_index: 4,
                    kind: StreamKind::Audio,
                    codec: Some("wmav2".to_owned()),
                },
                &media,
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
        let feedback = remux_ready_feedback(
            &RemuxValidationError::IncompatibleStream {
                container: MediaContainer::Mp4,
                stream_index: 15,
                kind: StreamKind::Data,
                codec: Some("bin_data".to_owned()),
            },
            &media,
        )
        .unwrap();

        assert_eq!(
            feedback,
            "Stream 15 (data, BIN_DATA) cannot be copied into MP4. Neither MP4 nor MKV can copy every source stream."
        );
        assert!(!feedback.contains("Use MKV instead"));
    }

    #[test]
    fn compact_inventory_summary_counts_each_retained_kind() {
        let media = media_with_streams(vec![
            MediaStream {
                stream_index: 0,
                kind: StreamKind::Video,
                codec: Some("h264".to_owned()),
            },
            MediaStream {
                stream_index: 2,
                kind: StreamKind::Audio,
                codec: Some("aac".to_owned()),
            },
            MediaStream {
                stream_index: 3,
                kind: StreamKind::Audio,
                codec: Some("opus".to_owned()),
            },
            MediaStream {
                stream_index: 8,
                kind: StreamKind::Subtitle,
                codec: Some("subrip".to_owned()),
            },
            MediaStream {
                stream_index: 9,
                kind: StreamKind::Attachment,
                codec: Some("ttf".to_owned()),
            },
        ]);
        assert_eq!(
            inventory_summary(&media).as_deref(),
            Some("1 video · 2 audio · 1 subtitle · 1 attachment")
        );
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

    #[test]
    fn constrained_ready_surface_selects_outer_scrolling() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let output = context.run_ui(Default::default(), |ui| {
            let media = media();
            let fixed = remux_fixed_flow_height(ui, &media, None);
            let plan = layout::task_surface_plan(fixed - 1.0, fixed, 0.0, 0.0);
            assert_eq!(plan.overflow, layout::TaskOverflow::Scroll);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn narrow_feedback_height_drives_ready_surface_overflow() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let output = context.run_ui(Default::default(), |ui| {
            let media = media_with_streams(vec![MediaStream {
                stream_index: 4,
                kind: StreamKind::Audio,
                codec: Some("wmav2".to_owned()),
            }]);
            let feedback = remux_ready_feedback(
                &RemuxValidationError::IncompatibleStream {
                    container: MediaContainer::Mp4,
                    stream_index: 4,
                    kind: StreamKind::Audio,
                    codec: Some("wmav2".to_owned()),
                },
                &media,
            )
            .unwrap();
            let narrow = fixed_height_ui(ui, 344.0);
            assert_eq!(narrow.available_width(), 344.0);
            let body_line = narrow.text_style_height(&egui::TextStyle::Body);
            let measured_feedback = crate::ui::widgets::inline_status_height(
                &narrow,
                &feedback,
                narrow.available_width(),
            );
            assert!(measured_feedback > body_line);

            let without_feedback = remux_fixed_flow_height(&narrow, &media, None);
            let old_one_line_estimate = without_feedback + theme::SPACING.tight + body_line;
            let with_feedback = remux_fixed_flow_height(&narrow, &media, Some(&feedback));
            assert!(with_feedback > old_one_line_estimate);
            assert_eq!(
                layout::task_surface_plan(old_one_line_estimate, with_feedback, 0.0, 0.0).overflow,
                layout::TaskOverflow::Scroll
            );
            assert_eq!(
                layout::task_surface_plan(with_feedback, with_feedback, 0.0, 0.0).overflow,
                layout::TaskOverflow::Flow
            );
            assert_eq!(
                layout::task_surface_plan(without_feedback - 1.0, without_feedback, 0.0, 0.0,)
                    .overflow,
                layout::TaskOverflow::Scroll
            );
            assert_eq!(
                layout::task_surface_plan(without_feedback, without_feedback, 0.0, 0.0,).overflow,
                layout::TaskOverflow::Flow
            );

            let wide = fixed_height_ui(ui, 800.0);
            let wide_without = remux_fixed_flow_height(&wide, &media, None);
            let wide_with = remux_fixed_flow_height(&wide, &media, Some(&feedback));
            let expected_wide_delta =
                theme::SPACING.tight + wide.text_style_height(&egui::TextStyle::Body);
            assert!((wide_with - wide_without - expected_wide_delta).abs() <= 0.5);
        });
        output.drop_without_applying_deltas();
    }

    fn media_with_streams(streams: Vec<MediaStream>) -> MediaFile {
        let mut media = media();
        media.streams = streams;
        media
    }

    fn fixed_height_ui(parent: &mut egui::Ui, width: f32) -> egui::Ui {
        let (rect, _) =
            parent.allocate_exact_size(egui::vec2(width, 1_000.0), egui::Sense::hover());
        parent.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        )
    }
}

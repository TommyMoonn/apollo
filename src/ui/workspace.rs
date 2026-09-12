use std::{cell::Cell, path::Path};

use egui::{Label, RichText, Sides, TextStyle, Ui};

use crate::media::model::MediaFile;

use super::{SourceAction, fonts, layout, media_lifecycle, queue_count_label, theme, widgets};

const CHANGE_FILE_LABEL: &str = "Change file";
const CLOSE_SOURCE_LABEL: &str = "Close source";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    AudioMix,
    VideoUpscale,
    Remux,
    AudioExtract,
    FrameCapture,
}

impl Operation {
    pub const ALL: [Self; 5] = [
        Self::AudioMix,
        Self::VideoUpscale,
        Self::Remux,
        Self::AudioExtract,
        Self::FrameCapture,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::AudioMix => "Audio Mix",
            Self::VideoUpscale => "Upscale",
            Self::Remux => "Remux",
            Self::AudioExtract => "Audio Extract",
            Self::FrameCapture => "Frame Capture",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceAction {
    None,
    Source(SourceAction),
    SelectOperation(Operation),
    OpenQueue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePresentation {
    file_name: String,
    metadata: Option<String>,
}

impl WorkspacePresentation {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            file_name: source_file_name(&media.path),
            metadata: source_metadata(media),
        }
    }
}

pub fn show(
    ui: &mut Ui,
    presentation: &WorkspacePresentation,
    active_operation: Operation,
    queue_count: usize,
    enabled: bool,
) -> WorkspaceAction {
    let action = Cell::new(WorkspaceAction::None);

    ui.add_enabled_ui(enabled, |ui| {
        show_source_header(ui, presentation, queue_count, &action);
        ui.add_space(theme::SPACING.related);
        show_operation_switcher(ui, active_operation, &action);
    });

    action.get()
}

fn show_source_header(
    ui: &mut Ui,
    presentation: &WorkspacePresentation,
    queue_count: usize,
    action: &Cell<WorkspaceAction>,
) {
    let queue_label = queue_count_label(queue_count);
    let action_sizes = [
        widgets::plain_button_intrinsic_size(ui, &queue_label),
        widgets::plain_button_intrinsic_size(ui, CHANGE_FILE_LABEL),
        widgets::plain_button_intrinsic_size(ui, CLOSE_SOURCE_LABEL),
    ];
    let action_widths = action_sizes.map(|size| size.x);
    let show_metadata = presentation.metadata.is_some()
        && layout::workspace_header_shows_metadata(ui.available_width(), &action_widths);
    let header_height = source_header_height(ui, show_metadata);

    Sides::new()
        .height(header_height)
        .spacing(theme::SOURCE_HEADER_GAP)
        .shrink_left()
        .truncate()
        .show(
            ui,
            |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = if show_metadata {
                        theme::SPACING.tight
                    } else {
                        0.0
                    };
                    ui.add(
                        Label::new(
                            RichText::new(&presentation.file_name).text_style(TextStyle::Body),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    if show_metadata && let Some(metadata) = presentation.metadata.as_deref() {
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
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::INLINE_ACTION_GAP;
                    if widgets::plain_button(ui, &queue_label).clicked() {
                        action.set(WorkspaceAction::OpenQueue);
                    }
                    if widgets::plain_button(ui, CHANGE_FILE_LABEL).clicked() {
                        action.set(WorkspaceAction::Source(SourceAction::ChangeFile));
                    }
                    if widgets::plain_button(ui, CLOSE_SOURCE_LABEL).clicked() {
                        action.set(WorkspaceAction::Source(SourceAction::CloseSource));
                    }
                });
            },
        );
}

fn show_operation_switcher(
    ui: &mut Ui,
    active_operation: Operation,
    action: &Cell<WorkspaceAction>,
) {
    let widths = Operation::ALL
        .map(|operation| widgets::mode_button_intrinsic_size(ui, operation.label()).x);
    if layout::workspace_switcher_fits_one_line(ui.available_width(), &widths) {
        show_operation_row(ui, &Operation::ALL, active_operation, action);
    } else {
        show_operation_row(ui, &Operation::ALL[..3], active_operation, action);
        ui.add_space(theme::SPACING.compact);
        show_operation_row(ui, &Operation::ALL[3..], active_operation, action);
    }
}

fn show_operation_row(
    ui: &mut Ui,
    operations: &[Operation],
    active_operation: Operation,
    action: &Cell<WorkspaceAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACING.related;
        for &operation in operations {
            if widgets::mode_button(ui, operation.label(), operation == active_operation).clicked()
            {
                action.set(WorkspaceAction::SelectOperation(operation));
            }
        }
    });
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
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Untitled media".to_owned())
}

fn source_metadata(media: &MediaFile) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(video) = media.video_streams.first()
        && let (Some(width), Some(height)) = (video.width, video.height)
        && width > 0
        && height > 0
    {
        parts.push(format!("{width}×{height}"));
    }
    if let Some(duration) = media.duration {
        parts.push(media_lifecycle::format_duration(duration));
    }
    if parts.is_empty() && !media.streams.is_empty() {
        parts.push(format!(
            "{} {}",
            media.streams.len(),
            if media.streams.len() == 1 {
                "stream"
            } else {
                "streams"
            }
        ));
    }

    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use crate::{
        media::model::{AudioStream, MediaFile, MediaStream, StreamKind, VideoStream},
        ui::{fonts, theme},
    };

    use super::{Operation, WorkspaceAction, WorkspacePresentation, show, source_metadata};

    fn shape_text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.job.text == text => {
                Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
            }
            egui::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| shape_text_rect(shape, text))
            }
            _ => None,
        }
    }

    fn output_text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| shape_text_rect(&shape.shape, text))
            .unwrap_or_else(|| panic!("missing text {text:?}"))
    }

    fn pointer_button_input(position: egui::Pos2, pressed: bool) -> egui::RawInput {
        egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        }
    }

    fn click_disabled_workspace_control(label: &str) -> WorkspaceAction {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut initial = context.run_ui(Default::default(), |ui| {
            let _ = show(ui, &presentation(), Operation::AudioMix, 3, false);
        });
        let position = output_text_rect(&initial, label).center();
        initial.textures_delta.clear();

        let mut pressed_action = WorkspaceAction::None;
        let mut pressed = context.run_ui(pointer_button_input(position, true), |ui| {
            pressed_action = show(ui, &presentation(), Operation::AudioMix, 3, false);
        });
        pressed.textures_delta.clear();
        assert_eq!(pressed_action, WorkspaceAction::None);

        let mut released_action = WorkspaceAction::None;
        let mut released = context.run_ui(pointer_button_input(position, false), |ui| {
            released_action = show(ui, &presentation(), Operation::AudioMix, 3, false);
        });
        released.textures_delta.clear();
        released_action
    }

    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("recording.mkv"),
            duration: Some(Duration::from_secs(151)),
            streams: vec![
                MediaStream {
                    stream_index: 0,
                    kind: StreamKind::Video,
                    codec: Some("h264".to_owned()),
                },
                MediaStream {
                    stream_index: 1,
                    kind: StreamKind::Audio,
                    codec: Some("aac".to_owned()),
                },
            ],
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: vec![AudioStream {
                stream_index: 1,
                codec: Some("aac".to_owned()),
                channels: Some(2),
                channel_layout: Some("stereo".to_owned()),
                title: None,
                language: None,
            }],
        }
    }

    fn presentation() -> WorkspacePresentation {
        WorkspacePresentation::new(&media())
    }

    #[test]
    fn shared_metadata_uses_source_dimensions_and_duration() {
        assert_eq!(
            source_metadata(&media()).as_deref(),
            Some("1920×1080 · 02:31")
        );
    }

    #[test]
    fn workspace_chrome_renders_all_operations_and_source_actions() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let output = context.run_ui(Default::default(), |ui| {
            assert_eq!(
                show(ui, &presentation(), Operation::Remux, 3, true),
                WorkspaceAction::None
            );
        });
        let debug = format!("{:?}", output.shapes);

        for label in [
            "recording.mkv",
            "Queue (3)",
            "Change file",
            "Close source",
            "Audio Mix",
            "Upscale",
            "Remux",
            "Audio Extract",
            "Frame Capture",
        ] {
            assert!(debug.contains(label), "missing {label:?}");
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn disabled_workspace_rejects_operation_and_source_actions() {
        assert_eq!(
            click_disabled_workspace_control("Upscale"),
            WorkspaceAction::None
        );
        assert_eq!(
            click_disabled_workspace_control("Change file"),
            WorkspaceAction::None
        );
    }

    #[test]
    fn narrow_workspace_keeps_actions_usable_and_reflows_operations_without_overflow() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let width = 344.0;
        let mut output = context.run_ui(Default::default(), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 260.0), egui::Sense::hover());
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            let _ = show(&mut child, &presentation(), Operation::AudioMix, 3, true);
        });

        let debug = format!("{:?}", output.shapes);
        assert!(!debug.contains("1920×1080"));
        let first = output_text_rect(&output, "Audio Mix");
        let last = output_text_rect(&output, "Frame Capture");
        output.textures_delta.clear();
        assert!(
            last.top() > first.top(),
            "operation rows did not reflow: first={first:?}, last={last:?}"
        );
        for label in Operation::ALL.map(Operation::label) {
            let rect = output_text_rect(&output, label);
            assert!(rect.left() >= 0.0);
            assert!(rect.right() <= width + 0.5, "{label} overflowed: {rect:?}");
        }
        for label in ["Queue (3)", "Change file", "Close source"] {
            assert!(output_text_rect(&output, label).right() <= width + 0.5);
        }
    }

    #[test]
    fn wide_workspace_keeps_all_operations_on_one_line() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(564.0, 180.0), egui::Sense::hover());
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            let _ = show(&mut child, &presentation(), Operation::AudioMix, 3, true);
        });

        let first = output_text_rect(&output, "Audio Mix");
        let last = output_text_rect(&output, "Frame Capture");
        output.textures_delta.clear();
        assert_eq!(first.top(), last.top());
    }
}

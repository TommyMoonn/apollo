use std::path::{Path, PathBuf};

use egui::{ColorImage, Image, TextureHandle, TextureOptions, Ui, vec2};

use crate::{
    frame_capture::{
        execution::{
            FrameCaptureExecutionFailure, FrameCaptureExecutionState, FrameCaptureExecutor,
        },
        model::FrameCaptureRequest,
        thumbnail::FrameCaptureThumbnail,
    },
    media::{
        error::{MediaError, MediaOutputContainer},
        progress::MediaProgress,
    },
};

use super::{
    frame_capture::format_label,
    layout, media_error,
    media_lifecycle::{
        file_name, heading, output_parent, progress_presentation, right_aligned_actions,
        secondary_copy, show_progress, source_to_output, technical_faint,
    },
    theme, widgets,
};

const CANCEL_LABEL: &str = "Cancel";
const RUNNING_TITLE: &str = "Capturing frame";
const COMPLETE_TITLE: &str = "Frame capture complete";
const FAILED_TITLE: &str = "Frame capture failed";
const CANCELLED_TITLE: &str = "Frame capture cancelled";
const OPEN_FOLDER_LABEL: &str = "Open folder";
const DONE_LABEL: &str = "Done";
const BACK_LABEL: &str = "Back";
const CANCELLED_COPY: &str = "No final output was published.";
const CANCELLED_CLEANUP: &str = "temporary output cleaned";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCaptureThumbnailRequest {
    request_id: u64,
    output: PathBuf,
}

impl FrameCaptureThumbnailRequest {
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn output(&self) -> &Path {
        &self.output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FrameCaptureThumbnailState {
    #[default]
    Empty,
    Loading {
        request_id: u64,
    },
    Ready {
        request_id: u64,
        preview: FrameCaptureThumbnail,
    },
    Failed {
        request_id: u64,
        diagnostic: String,
    },
}

#[derive(Clone, PartialEq, Eq)]
struct FrameCaptureThumbnailTexture {
    request_id: u64,
    handle: TextureHandle,
}

impl std::fmt::Debug for FrameCaptureThumbnailTexture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrameCaptureThumbnailTexture")
            .field("request_id", &self.request_id)
            .field("texture_id", &self.handle.id())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameCaptureLifecycleUiState {
    details_open: bool,
    next_thumbnail_request_id: u64,
    thumbnail: FrameCaptureThumbnailState,
    thumbnail_texture: Option<FrameCaptureThumbnailTexture>,
}

impl FrameCaptureLifecycleUiState {
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    pub fn thumbnail(&self) -> &FrameCaptureThumbnailState {
        &self.thumbnail
    }

    pub fn settle_thumbnail(
        &mut self,
        request_id: u64,
        result: Result<FrameCaptureThumbnail, String>,
    ) -> bool {
        if !matches!(
            self.thumbnail,
            FrameCaptureThumbnailState::Loading {
                request_id: active_request_id
            } if active_request_id == request_id
        ) {
            return false;
        }

        self.thumbnail = match result {
            Ok(preview) => FrameCaptureThumbnailState::Ready {
                request_id,
                preview,
            },
            Err(diagnostic) => FrameCaptureThumbnailState::Failed {
                request_id,
                diagnostic,
            },
        };
        self.thumbnail_texture = None;
        true
    }

    pub fn begin_thumbnail(&mut self, output: &Path) -> Option<FrameCaptureThumbnailRequest> {
        if !matches!(self.thumbnail, FrameCaptureThumbnailState::Empty) {
            return None;
        }

        self.next_thumbnail_request_id = self
            .next_thumbnail_request_id
            .checked_add(1)
            .expect("frame capture thumbnail request identifier space is exhausted");
        let request_id = self.next_thumbnail_request_id;
        self.thumbnail = FrameCaptureThumbnailState::Loading { request_id };
        self.thumbnail_texture = None;
        Some(FrameCaptureThumbnailRequest {
            request_id,
            output: output.to_path_buf(),
        })
    }

    fn reset(&mut self) {
        self.details_open = false;
        self.thumbnail = FrameCaptureThumbnailState::Empty;
        self.thumbnail_texture = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FrameCaptureLifecycleAction {
    #[default]
    None,
    CancelRequested,
    LoadThumbnail(FrameCaptureThumbnailRequest),
    OpenFolder(PathBuf),
    Done,
    Back,
}

pub fn show(
    ui: &mut Ui,
    request: &FrameCaptureRequest,
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
) -> FrameCaptureLifecycleAction {
    let execution_state = executor.poll().clone();
    let progress = executor.progress().cloned();

    match execution_state {
        FrameCaptureExecutionState::Idle => FrameCaptureLifecycleAction::None,
        FrameCaptureExecutionState::Running => {
            show_running(ui, request, progress.as_ref(), executor)
        }
        FrameCaptureExecutionState::Completed { output } => {
            let thumbnail_request = (!ui.is_sizing_pass())
                .then(|| ui_state.begin_thumbnail(&output))
                .flatten();
            let action = show_completed(ui, request, &output, executor, ui_state);
            if matches!(action, FrameCaptureLifecycleAction::None) {
                thumbnail_request
                    .map(FrameCaptureLifecycleAction::LoadThumbnail)
                    .unwrap_or_default()
            } else {
                action
            }
        }
        FrameCaptureExecutionState::Failed(failure) => {
            show_failed(ui, &failure, executor, ui_state)
        }
        FrameCaptureExecutionState::Cancelled => show_cancelled(ui, executor, ui_state),
    }
}

fn show_running(
    ui: &mut Ui,
    request: &FrameCaptureRequest,
    progress: Option<&MediaProgress>,
    executor: &FrameCaptureExecutor,
) -> FrameCaptureLifecycleAction {
    let mut action = FrameCaptureLifecycleAction::None;
    let presentation = progress_presentation(progress);
    let path_summary = source_to_output(&request.input, &request.output);
    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, RUNNING_TITLE);
        ui.add_space(theme::STATUS_COPY_GAP);
        secondary_copy(ui, &path_summary);
        ui.add_space(theme::SPACING.spacious);
        show_progress(ui, &presentation);
        ui.add_space(theme::RUNNING_FOOTER_GAP);
        let response =
            right_aligned_actions(ui, |ui| widgets::destructive_button(ui, CANCEL_LABEL));
        if !ui.is_sizing_pass() && response.clicked() && executor.cancel() {
            action = FrameCaptureLifecycleAction::CancelRequested;
        }
    });
    action
}

fn show_completed(
    ui: &mut Ui,
    request: &FrameCaptureRequest,
    output: &Path,
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
) -> FrameCaptureLifecycleAction {
    let mut action = FrameCaptureLifecycleAction::None;
    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, COMPLETE_TITLE);
        ui.add_space(theme::SPACING.focal);
        show_thumbnail(ui, ui_state);
        ui.add_space(theme::SPACING.spacious);
        file_name(ui, output);
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, format_label(request.format));
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, &request.timestamp.to_string());
        ui.add_space(theme::RESULT_FOOTER_GAP);
        let (done, open_folder) = right_aligned_actions(ui, |ui| {
            ui.spacing_mut().item_spacing.x = theme::FOOTER_ACTION_GAP;
            let done = widgets::primary_button(ui, DONE_LABEL);
            let open_folder = widgets::secondary_button(ui, OPEN_FOLDER_LABEL);
            (done, open_folder)
        });
        if !ui.is_sizing_pass() {
            if done.clicked() {
                reset_terminal(executor, ui_state, "completed");
                action = FrameCaptureLifecycleAction::Done;
            } else if open_folder.clicked() {
                action = FrameCaptureLifecycleAction::OpenFolder(output_parent(output));
            }
        }
    });
    action
}

fn show_thumbnail(ui: &mut Ui, ui_state: &mut FrameCaptureLifecycleUiState) {
    let (thumbnail, thumbnail_texture) = (&ui_state.thumbnail, &mut ui_state.thumbnail_texture);
    match thumbnail {
        FrameCaptureThumbnailState::Empty => {}
        FrameCaptureThumbnailState::Loading { .. } => {
            technical_faint(ui, "Loading preview...");
        }
        FrameCaptureThumbnailState::Failed { .. } => {
            technical_faint(ui, "Preview unavailable");
        }
        FrameCaptureThumbnailState::Ready {
            request_id,
            preview,
        } => {
            let request_id = *request_id;
            if !ui.is_sizing_pass()
                && thumbnail_texture
                    .as_ref()
                    .is_none_or(|texture| texture.request_id != request_id)
            {
                let image = ColorImage::from_rgba_unmultiplied(preview.size(), preview.rgba());
                let handle = ui.ctx().load_texture(
                    format!("frame-capture-thumbnail-{request_id}"),
                    image,
                    TextureOptions::LINEAR,
                );
                *thumbnail_texture = Some(FrameCaptureThumbnailTexture { request_id, handle });
            }

            let [width, height] = preview.size();
            let display_width = (width as f32).min(ui.available_width());
            let display_size = vec2(display_width, display_width * height as f32 / width as f32);
            if let Some(texture) = thumbnail_texture.as_ref() {
                ui.add(
                    Image::new((texture.handle.id(), display_size))
                        .alt_text("Captured frame preview"),
                );
            } else {
                ui.allocate_space(display_size);
            }
        }
    }
}

fn show_failed(
    ui: &mut Ui,
    failure: &FrameCaptureExecutionFailure,
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
) -> FrameCaptureLifecycleAction {
    let error = MediaError::from_execution_failure(failure, MediaOutputContainer::Other);
    let response = layout::focus_content(ui, |ui| {
        let mut sizing_details_open = ui_state.details_open;
        let details_open = if ui.is_sizing_pass() {
            &mut sizing_details_open
        } else {
            &mut ui_state.details_open
        };
        media_error::show_failure_content(ui, FAILED_TITLE, &error, details_open, BACK_LABEL)
    });
    if response.inner.clicked() {
        reset_terminal(executor, ui_state, "failed");
        FrameCaptureLifecycleAction::Back
    } else {
        FrameCaptureLifecycleAction::None
    }
}

fn show_cancelled(
    ui: &mut Ui,
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
) -> FrameCaptureLifecycleAction {
    let mut action = FrameCaptureLifecycleAction::None;
    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, CANCELLED_TITLE);
        ui.add_space(theme::STATUS_COPY_GAP);
        secondary_copy(ui, CANCELLED_COPY);
        ui.add_space(theme::DIAGNOSTIC_TOP_GAP);
        technical_faint(ui, CANCELLED_CLEANUP);
        ui.add_space(theme::RESULT_FOOTER_GAP);
        let response = right_aligned_actions(ui, |ui| widgets::primary_button(ui, DONE_LABEL));
        if !ui.is_sizing_pass() && response.clicked() {
            reset_terminal(executor, ui_state, "cancelled");
            action = FrameCaptureLifecycleAction::Done;
        }
    });
    action
}

fn reset_terminal(
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
    terminal_name: &str,
) {
    let reset = executor.reset();
    debug_assert!(reset, "{terminal_name} executor must be resettable");
    ui_state.reset();
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::frame_capture::{
        execution::FrameCaptureExecutor,
        model::{FrameCaptureFormat, FrameCaptureRequest, FrameTimestamp},
        thumbnail::FrameCaptureThumbnail,
    };

    use super::{
        CANCELLED_CLEANUP, CANCELLED_COPY, CANCELLED_TITLE, COMPLETE_TITLE, FAILED_TITLE,
        FrameCaptureLifecycleUiState, FrameCaptureThumbnailState, RUNNING_TITLE, show_cancelled,
        show_completed, show_running,
    };

    #[test]
    fn lifecycle_copy_matches_frame_capture_surfaces() {
        assert_eq!(RUNNING_TITLE, "Capturing frame");
        assert_eq!(COMPLETE_TITLE, "Frame capture complete");
        assert_eq!(FAILED_TITLE, "Frame capture failed");
        assert_eq!(CANCELLED_TITLE, "Frame capture cancelled");
    }

    #[test]
    fn running_completed_and_cancelled_views_render_expected_content() {
        let request = request();
        let executor = FrameCaptureExecutor::new();
        let running = render(|ui| {
            let _ = show_running(ui, &request, None, &executor);
        });
        for text in [
            "Capturing frame",
            "recording.mkv → recording-frame.png",
            "Processing",
            "Cancel",
        ] {
            assert!(contains(&running, text), "missing {text:?}");
        }
        running.drop_without_applying_deltas();

        let mut executor = FrameCaptureExecutor::new();
        let mut state = FrameCaptureLifecycleUiState::default();
        let completed = render(|ui| {
            let _ = show_completed(ui, &request, &request.output, &mut executor, &mut state);
        });
        for text in [
            "Frame capture complete",
            "recording-frame.png",
            "PNG",
            "00:00:01.250",
            "Open folder",
            "Done",
        ] {
            assert!(contains(&completed, text), "missing {text:?}");
        }
        completed.drop_without_applying_deltas();

        let cancelled = render(|ui| {
            let _ = show_cancelled(ui, &mut executor, &mut state);
        });
        for text in [CANCELLED_TITLE, CANCELLED_COPY, CANCELLED_CLEANUP, "Done"] {
            assert!(contains(&cancelled, text), "missing {text:?}");
        }
        cancelled.drop_without_applying_deltas();
    }

    #[test]
    fn completed_capture_starts_exactly_one_thumbnail_request() {
        let mut state = FrameCaptureLifecycleUiState::default();
        let output = PathBuf::from("C:/media/recording-frame.png");

        let request = state.begin_thumbnail(&output).unwrap();
        assert_eq!(request.request_id(), 1);
        assert_eq!(request.output(), output);
        assert!(state.begin_thumbnail(&output).is_none());
        assert!(matches!(
            state.thumbnail(),
            FrameCaptureThumbnailState::Loading { request_id: 1 }
        ));
    }

    #[test]
    fn stale_thumbnail_completion_cannot_replace_a_newer_capture() {
        let mut state = FrameCaptureLifecycleUiState::default();
        let first = state
            .begin_thumbnail(PathBuf::from("first.png").as_path())
            .unwrap();
        state.reset();
        let second = state
            .begin_thumbnail(PathBuf::from("second.png").as_path())
            .unwrap();

        assert!(!state.settle_thumbnail(first.request_id(), Ok(preview([2, 1]))));
        assert!(matches!(
            state.thumbnail(),
            FrameCaptureThumbnailState::Loading { request_id } if *request_id == second.request_id()
        ));
        assert!(state.settle_thumbnail(second.request_id(), Ok(preview([2, 1]))));
        assert!(matches!(
            state.thumbnail(),
            FrameCaptureThumbnailState::Ready { request_id, .. }
                if *request_id == second.request_id()
        ));
    }

    #[test]
    fn thumbnail_decode_failure_keeps_completed_capture_surface_successful() {
        let request = request();
        let mut executor = FrameCaptureExecutor::new();
        let mut state = FrameCaptureLifecycleUiState::default();
        let thumbnail = state.begin_thumbnail(&request.output).unwrap();
        assert!(
            state.settle_thumbnail(thumbnail.request_id(), Err("unsupported image".to_owned()))
        );

        let completed = render(|ui| {
            let _ = show_completed(ui, &request, &request.output, &mut executor, &mut state);
        });
        assert!(contains(&completed, COMPLETE_TITLE));
        assert!(contains(&completed, "Preview unavailable"));
        assert!(!contains(&completed, FAILED_TITLE));
        completed.drop_without_applying_deltas();
    }

    #[test]
    fn ready_thumbnail_texture_is_reused_across_unrelated_repaints() {
        let context = egui::Context::default();
        context.set_fonts(egui::FontDefinitions::default());
        let request = request();
        let mut executor = FrameCaptureExecutor::new();
        let mut state = FrameCaptureLifecycleUiState::default();
        let thumbnail = state.begin_thumbnail(&request.output).unwrap();
        assert!(state.settle_thumbnail(thumbnail.request_id(), Ok(preview([4, 2]))));

        let first = render_with_context(&context, |ui| {
            let _ = show_completed(ui, &request, &request.output, &mut executor, &mut state);
        });
        let texture_id = state.thumbnail_texture.as_ref().unwrap().handle.id();
        assert!(!first.textures_delta.set.is_empty());
        first.drop_without_applying_deltas();

        let second = render_with_context(&context, |ui| {
            let _ = show_completed(ui, &request, &request.output, &mut executor, &mut state);
        });
        assert_eq!(
            state.thumbnail_texture.as_ref().unwrap().handle.id(),
            texture_id
        );
        assert!(second.textures_delta.set.is_empty());
        second.drop_without_applying_deltas();
    }

    fn preview(size: [usize; 2]) -> FrameCaptureThumbnail {
        FrameCaptureThumbnail::from_rgba(size, vec![255; size[0] * size[1] * 4]).unwrap()
    }

    fn request() -> FrameCaptureRequest {
        FrameCaptureRequest {
            input: PathBuf::from("C:/media/recording.mkv"),
            output: PathBuf::from("C:/media/recording-frame.png"),
            timestamp: FrameTimestamp::from_milliseconds(1_250),
            format: FrameCaptureFormat::Png,
        }
    }

    fn render(add: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let context = egui::Context::default();
        context.set_fonts(egui::FontDefinitions::default());
        render_with_context(&context, add)
    }

    fn render_with_context(
        context: &egui::Context,
        mut add: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        context.run_ui(Default::default(), |ui| {
            let mut style = (**ui.style()).clone();
            style.text_styles.insert(
                crate::ui::fonts::technical_text_style(),
                egui::FontId::new(12.0, egui::FontFamily::Monospace),
            );
            style.text_styles.insert(
                crate::ui::fonts::wordmark_text_style(),
                egui::FontId::new(26.0, egui::FontFamily::Monospace),
            );
            ui.set_style(style);
            add(ui);
        })
    }

    fn contains(output: &egui::FullOutput, expected: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|clipped| shape_contains(&clipped.shape, expected))
    }

    fn shape_contains(shape: &egui::Shape, expected: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text.contains(expected),
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| shape_contains(shape, expected)),
            _ => false,
        }
    }
}

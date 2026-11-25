use crate::app::frame_capture::frame_navigation_availability;
use crate::app::frame_capture::{FrameCaptureThumbnailLoad, FramePreviewDecode};
use crate::frame_capture::draft::FrameCaptureDraft;
mod dialogs;
mod frame_capture;
mod navigation;
mod source;

use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use super::state::{
    BatchDefaults, BatchEditTarget, BatchItem, BatchSetupState, BatchSourceState,
    FrameCaptureToolState, QueueExecutionEvent,
};
use super::workflows::{
    completed_output_folder, queue_event_matches_active, queue_execution_wake, refresh_batch_items,
    shows_audio_extract_lifecycle, shows_audio_mix_lifecycle, shows_frame_capture_lifecycle,
    shows_remux_lifecycle, shows_upscale_lifecycle,
};
use super::{ApolloApp, completion_wake};
use crate::app::discovery::{MediaToolsDiscovery, MediaToolsState};
use crate::app::{
    Application, BatchOrigin, NavigationState, Operation, QueueEditOrigin,
    operation_workspace_visible,
};
use crate::frame_capture::clipboard::copy_frame_action_eligible;
use crate::frame_capture::frame_index::{
    FrameDirection, FrameIndexError, FrameIndexState, FrameStep,
};
use crate::settings::AppSettings;
use crate::ui::frame_capture::PositionSettle;

use crate::app::dialogs::{DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogRequest};
use crate::app::{QueueOrigin, file_intake_enabled};
use crate::file_dialog::FileDialogOwner;
use crate::{
    audio_extract::{execution::AudioExtractExecutionState, model::AudioExtractFormat},
    audio_mix::{
        execution::AudioMixExecutionState,
        model::{LoudnessNormalization, LoudnessTargets, OutputContainer, TrackGain},
    },
    frame_capture::{
        execution::FrameCaptureExecutionState,
        model::{
            FrameCaptureFormat, FramePosition, FrameTimeBase, FrameTimestamp, ResolvedFramePosition,
        },
        preview::{PREVIEW_DEBOUNCE, PreviewImage, PreviewState},
        thumbnail::FrameCaptureThumbnail,
    },
    media::{
        container::MediaContainer,
        error::{MediaError, MediaErrorKind},
        execution::MediaExecutionFailure,
        intake::{MediaIntake, MediaLoadStart},
        model::{AudioStream, FrameRate, MediaFile, MediaStream, StreamKind, VideoStream},
        probe::MediaProbeError,
        process::{MediaProcessEvent, MediaProcessFailure, MediaProcessWake},
        tools::{MediaTools, MediaToolsError},
    },
    queue::{QueueJobFailure, QueueJobState, QueueRunState, QueuedOperation},
    remux::execution::RemuxExecutionState,
    ui::{
        QueueAction, SourceAction, audio_extract::AudioExtractSelectionAction,
        audio_mix::AudioMixSelectionAction, batch::BatchReadiness, file_intake::FileIntakeAction,
        fonts, frame_capture::FrameCaptureSelectionAction,
        frame_capture_progress::FrameCaptureThumbnailState, layout, media_error,
        remux::RemuxSelectionAction, theme, upscale::UpscaleSelectionAction,
    },
    upscale::{
        execution::UpscaleExecutionState,
        model::{UpscaleTarget, VideoCodec},
    },
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

#[test]
fn pending_capabilities_render_in_the_existing_workspace_and_settle_without_reset() {
    use crate::media::tools::{VideoEncoderCapabilities, VideoEncoderCapability};
    use crate::upscale::model::{CpuQuality, VideoEncoderSelection};
    let (directory, path) = temp_file("pending-nvenc-workspace");
    let mut app = app();
    let context = app.egui_context.clone();
    fonts::install(&context);
    theme::install(&context);
    let (wake_tx, wake_rx) = mpsc::channel();
    context.set_request_repaint_callback(move |_| {
        let _ = wake_tx.send(());
    });
    let (release_tx, release_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    assert!(app.core.discovery.start_with(
        completion_wake(&context),
        || Ok(MediaTools::new("missing-ffmpeg", "missing-ffprobe")),
        move |_| {
            started_tx.send(std::thread::current().id()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            VideoEncoderCapabilities {
                h264_nvenc: VideoEncoderCapability::Available,
                h265_nvenc: VideoEncoderCapability::RuntimeUnavailable {
                    reason: "driver lacks HEVC".to_owned(),
                },
            }
        },
    ));
    assert_ne!(
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
        std::thread::current().id()
    );
    app.poll_background_work();
    assert!(app.ready_media_tools().is_some());
    assert!(app.core.discovery.is_pending());
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);
    let render = |app: &mut ApolloApp| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(730.0, 1000.0),
                )),
                ..Default::default()
            },
            |ui| app.show_navigation(ui),
        );
        output.textures_delta.clear();
        output
    };
    let output = render(&mut app);
    assert!(output_contains_text(&output, "Auto (checking...)"));
    assert!(output_contains_text(&output, "Checking NVIDIA NVENC..."));
    assert!(
        app.enqueue_current_operation(Operation::VideoUpscale)
            .unwrap()
            .is_none()
    );
    assert!(
        app.handle_upscale_action(UpscaleSelectionAction::UpscaleRequested)
            .is_ok()
    );
    assert!(matches!(
        app.core.upscale_executor.state(),
        UpscaleExecutionState::Idle
    ));
    app.state
        .upscale
        .as_mut()
        .unwrap()
        .selection
        .set_encoder_selection(VideoEncoderSelection::NvidiaNvenc);
    let output = render(&mut app);
    assert!(output_contains_text(&output, "Checking NVIDIA NVENC..."));
    assert!(
        app.enqueue_current_operation(Operation::VideoUpscale)
            .unwrap()
            .is_none()
    );
    assert!(
        app.handle_upscale_action(UpscaleSelectionAction::UpscaleRequested)
            .is_ok()
    );
    assert!(matches!(
        app.core.upscale_executor.state(),
        UpscaleExecutionState::Idle
    ));
    app.state
        .upscale
        .as_mut()
        .unwrap()
        .selection
        .set_encoder_selection(VideoEncoderSelection::Cpu);
    let job_id = app
        .enqueue_current_operation(Operation::VideoUpscale)
        .unwrap()
        .unwrap();
    assert!(app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
    app.queue_edit
        .as_mut()
        .unwrap()
        .upscale
        .set_encoder_selection(VideoEncoderSelection::Auto);
    assert!(!app.save_queue_edit());
    assert!(app.finish_queue_edit());
    // The missing executable produces a start error only after CPU passes ready validation.
    assert!(matches!(
        app.handle_upscale_action(UpscaleSelectionAction::UpscaleRequested),
        Err(crate::upscale::execution::UpscaleStartError::Process(_))
    ));
    let session = app.state.upscale.as_mut().unwrap();
    session
        .selection
        .set_encoder_selection(VideoEncoderSelection::Auto);
    session
        .selection
        .set_cpu_quality(VideoCodec::H264, CpuQuality::new(22).unwrap());
    session
        .selection
        .set_target(&session.media, UpscaleTarget::Uhd8k);
    session
        .selection
        .set_output_filename_stem("retained-output".into());
    let before = session.selection.clone();
    assert_eq!(
        settled_navigation_repaint_delay(&context, &mut app),
        Duration::MAX
    );
    // Batch defaults and overrides cannot bypass the pending capability gate.
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::SourceWorkspace {
            active_operation: Operation::VideoUpscale,
        },
        operation: Operation::VideoUpscale,
        defaults: None,
        items: vec![BatchItem {
            path: path.clone(),
            included: true,
            source: ready_batch_source(media(&path)),
            operation: None,
            readiness: BatchReadiness::Inspecting,
            reason: None,
            overridden: false,
        }],
        error: None,
        append_error: None,
        append_error_details_open: false,
    });
    app.core.show_batch_setup();
    refresh_batch_items(
        app.batch_setup.as_mut().unwrap(),
        app.core.discovery.upscale_capabilities(),
    );
    let item = &app.batch_setup.as_ref().unwrap().items[0];
    assert_eq!(item.readiness, BatchReadiness::NeedsAttention);
    assert!(item.operation.is_none());
    assert!(app.begin_batch_edit(BatchEditTarget::Item(0)));
    assert!(!app.save_batch_edit());
    assert!(app.finish_batch_edit());
    while wake_rx.try_recv().is_ok() {}
    app.tools_error_details_open = true;
    release_tx.send(()).unwrap();
    wake_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("capability completion wakes egui");
    app.poll_background_work();
    assert!(!app.core.discovery.is_pending());
    assert!(!app.tools_error_details_open);
    let item = &app.batch_setup.as_ref().unwrap().items[0];
    assert_eq!(item.readiness, BatchReadiness::Ready);
    assert!(
        matches!(item.operation.as_ref(), Some(QueuedOperation::Upscale(request)) if request.nvenc_capability == VideoEncoderCapability::Available)
    );
    assert!(app.finish_batch_setup());
    assert_eq!(app.state.upscale.as_ref().unwrap().selection, before);
    assert!(output_contains_text(
        &render(&mut app),
        "Auto (NVIDIA NVENC)"
    ));
    app.state
        .upscale
        .as_mut()
        .unwrap()
        .selection
        .set_codec(VideoCodec::H265);
    assert!(output_contains_text(&render(&mut app), "Auto (CPU)"));
    app.state
        .upscale
        .as_mut()
        .unwrap()
        .selection
        .set_encoder_selection(VideoEncoderSelection::NvidiaNvenc);
    assert!(output_contains_text(&render(&mut app), "driver lacks HEVC"));
    assert_eq!(
        settled_navigation_repaint_delay(&context, &mut app),
        Duration::MAX
    );
    fs::remove_dir_all(directory).unwrap();
}

#[derive(Debug)]
struct TestDroppedFile {
    path: PathBuf,
}

impl egui::DroppedFile for TestDroppedFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        fs::read(&self.path).map_err(|error| error.to_string())
    }
}

fn temp_file(label: &str) -> (PathBuf, PathBuf) {
    let directory = std::env::temp_dir().join(format!(
        "apollo-phase35-app-{label}-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("recording.mkv");
    fs::write(&path, b"media").unwrap();
    (directory, path)
}

fn media(path: &Path) -> MediaFile {
    MediaFile {
        path: path.to_path_buf(),
        duration: None,
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
            frame_rate: FrameRate::new(60, 1),
            format: Default::default(),
        }],
        audio_streams: vec![AudioStream {
            stream_index: 1,
            codec: Some("aac".to_owned()),
            channels: Some(2),
            channel_layout: Some("stereo".to_owned()),
            title: Some("Desktop Audio".to_owned()),
            language: None,
        }],
    }
}

fn thumbnail() -> FrameCaptureThumbnail {
    FrameCaptureThumbnail::from_rgba([2, 1], vec![255; 8]).unwrap()
}

fn app() -> ApolloApp {
    let egui_context = egui::Context::default();
    let file_dialog = FileDialogOwner::new_for_test(completion_wake(&egui_context), || {
        Ok(|_: &FileDialogRequest| Ok(None))
    });
    let mut app = ApolloApp::with_adapters(egui_context, file_dialog, None, AppSettings::default());
    app.core.discovery = MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
        "missing-ffmpeg",
        "missing-ffprobe",
    )));
    app
}

fn seed_frame_neighborhood(app: &mut ApolloApp, center: FramePosition, payload: &[u8]) {
    let (media, editor) = app.active_frame_capture_editor_mut().unwrap();
    let editor = &mut editor.state;
    editor.frame_index_due = None;
    editor.selection.set_position(media, center);
    let request = editor
        .frame_navigator
        .schedule(editor.source_identity.as_ref().unwrap(), media, center)
        .unwrap()
        .unwrap();
    assert!(editor.frame_navigator.settle(request, Ok(payload.to_vec())));
}

fn ready_batch_source(media: MediaFile) -> BatchSourceState {
    let mut intake = MediaIntake::default();
    let MediaLoadStart::Probe(request) = intake.begin_load(&media.path).unwrap() else {
        unreachable!("new batch test intake cannot reuse a source")
    };
    intake.complete_probe(request, Ok(media.clone())).unwrap();
    BatchSourceState::Ready {
        source_identity: intake.current_with_identity().unwrap().0.clone(),
        media,
    }
}

fn assert_no_batch_execution(app: &ApolloApp) {
    assert!(app.core.media_queue.jobs().is_empty());
    assert!(!app.core.audio_mix_executor.is_running());
    assert!(!app.core.upscale_executor.is_running());
    assert!(!app.core.remux_executor.is_running());
    assert!(!app.core.audio_extract_executor.is_running());
    assert!(!app.core.frame_capture_executor.is_running());
}

fn populate_cancelled_and_failed_queue_history(app: &mut ApolloApp, path: &Path, directory: &Path) {
    app.load_media_path_with_probe(path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    app.enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    assert!(app.core.media_queue.cancel_queue());
    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(directory.join("failed-output.mkv"));
    app.enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    assert!(!app.start_media_queue());
    assert_eq!(app.core.media_queue.pending_count(), 0);
    assert!(matches!(
        app.core.media_queue.jobs()[0].state(),
        QueueJobState::Cancelled
    ));
    assert!(matches!(
        app.core.media_queue.jobs()[1].state(),
        QueueJobState::Failed(_)
    ));
}

fn show_tool(app: &mut ApolloApp, ui: &mut egui::Ui, operation: Operation) {
    app.select_tool(operation);
    app.show_navigation(ui);
}

fn assert_no_source(state: &Application) {
    assert_eq!(state.navigation(), NavigationState::NoSource);
    assert!(state.source.current().is_none());
}

fn assert_workspace(state: &Application, operation: Operation) {
    assert_eq!(
        state.navigation(),
        NavigationState::SourceWorkspace {
            active_operation: operation,
        }
    );
    assert!(state.source.current().is_some());
}

fn dropped_file(path: &Path) -> egui::DroppedFileHandle {
    Arc::new(TestDroppedFile {
        path: path.to_path_buf(),
    })
}

fn handle_drop(app: &mut ApolloApp, paths: &[&Path]) {
    let context = egui::Context::default();
    let input = egui::RawInput {
        dropped_files: paths.iter().map(|path| dropped_file(path)).collect(),
        ..Default::default()
    };
    let output = context.run_ui(input, |ui| app.handle_window_file_drop(ui.ctx()));
    output.drop_without_applying_deltas();
}

fn drag_hovered(app: &ApolloApp, paths: &[&Path]) -> bool {
    let context = egui::Context::default();
    let input = egui::RawInput {
        hovered_files: paths
            .iter()
            .map(|path| egui::HoveredFile {
                path: Some((*path).to_path_buf()),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut hovered = false;
    let output = context.run_ui(input, |ui| hovered = app.drag_hovered(ui.ctx()));
    output.drop_without_applying_deltas();
    hovered
}

fn begin_pending_inspection(
    app: &mut ApolloApp,
    path: &Path,
) -> mpsc::SyncSender<Result<MediaFile, MediaProbeError>> {
    let request = app
        .begin_media_path(path)
        .expect("new test source should require inspection");
    let (sender, receiver) = mpsc::sync_channel(1);
    app.core.source.attach_inspection(request, receiver);
    sender
}

fn shape_contains_text(shape: &egui::Shape, needle: &str) -> bool {
    let mut pending = vec![shape];
    while let Some(shape) = pending.pop() {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(needle) => return true,
            egui::Shape::Vec(shapes) => pending.extend(shapes),
            _ => {}
        }
    }
    false
}

fn output_contains_text(output: &egui::FullOutput, needle: &str) -> bool {
    output
        .shapes
        .iter()
        .any(|clipped| shape_contains_text(&clipped.shape, needle))
}

fn settled_navigation_repaint_delay(context: &egui::Context, app: &mut ApolloApp) -> Duration {
    for _ in 0..8 {
        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
        let repaint_delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        output.textures_delta.clear();
        if repaint_delay == Duration::MAX {
            return repaint_delay;
        }
    }
    Duration::ZERO
}

fn shape_text_count(shape: &egui::Shape, needle: &str) -> usize {
    let mut count = 0;
    let mut pending = vec![shape];
    while let Some(shape) = pending.pop() {
        match shape {
            egui::Shape::Text(text) => count += usize::from(text.galley.job.text == needle),
            egui::Shape::Vec(shapes) => pending.extend(shapes),
            _ => {}
        }
    }
    count
}

fn output_text_count(output: &egui::FullOutput, needle: &str) -> usize {
    output
        .shapes
        .iter()
        .map(|shape| shape_text_count(&shape.shape, needle))
        .sum()
}

fn shape_text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
    let mut pending = vec![shape];
    while let Some(shape) = pending.pop() {
        match shape {
            egui::Shape::Text(shape) if shape.galley.job.text == text => {
                return Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()));
            }
            egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
            _ => {}
        }
    }
    None
}

fn output_text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|clipped| shape_text_rect(&clipped.shape, text))
        .unwrap_or_else(|| panic!("missing rendered text {text:?}"))
}

fn output_bottom_text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
    let mut matches = Vec::new();
    for clipped in &output.shapes {
        let mut pending = vec![&clipped.shape];
        while let Some(shape) = pending.pop() {
            match shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => {
                    matches.push(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
                }
                egui::Shape::Vec(shapes) => pending.extend(shapes),
                _ => {}
            }
        }
    }
    matches
        .into_iter()
        .max_by(|left, right| left.bottom().total_cmp(&right.bottom()))
        .unwrap_or_else(|| panic!("missing rendered text {text:?}"))
}

fn terminal_execution_states() -> [AudioMixExecutionState; 3] {
    [
        AudioMixExecutionState::Completed {
            output: PathBuf::from("finished.mp4"),
        },
        AudioMixExecutionState::Failed(MediaExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: "encoding failed".to_owned(),
        }),
        AudioMixExecutionState::Cancelled,
    ]
}

#[test]
fn audio_extract_batch_requires_explicit_selection_for_multi_track_sources() {
    let (first_directory, first_path) = temp_file("batch-audio-first");
    let (second_directory, second_path) = temp_file("batch-audio-second");
    let first_media = media(&first_path);
    let mut second_media = media(&second_path);
    second_media.streams.push(MediaStream {
        stream_index: 2,
        kind: StreamKind::Audio,
        codec: Some("aac".to_owned()),
    });
    second_media.audio_streams.push(AudioStream {
        stream_index: 2,
        codec: Some("aac".to_owned()),
        channels: Some(2),
        channel_layout: Some("stereo".to_owned()),
        title: Some("Commentary".to_owned()),
        language: Some("eng".to_owned()),
    });
    let mut batch = BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::AudioExtract,
        defaults: Some(BatchDefaults::AudioExtract {
            format: AudioExtractFormat::Original,
        }),
        items: vec![
            BatchItem {
                path: first_path,
                included: true,
                source: ready_batch_source(first_media),
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            },
            BatchItem {
                path: second_path,
                included: true,
                source: ready_batch_source(second_media),
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            },
        ],
        error: None,
        append_error: None,
        append_error_details_open: false,
    };

    refresh_batch_items(&mut batch, None);

    assert_eq!(batch.items[0].readiness, BatchReadiness::Ready);
    assert_eq!(batch.items[1].readiness, BatchReadiness::NeedsAttention);
    assert_eq!(
        batch.items[1].reason.as_deref(),
        Some("Select one audio track.")
    );
    assert_ne!(
        batch.items[0]
            .operation
            .as_ref()
            .map(QueuedOperation::output),
        batch.items[1]
            .operation
            .as_ref()
            .map(QueuedOperation::output)
    );
    fs::remove_dir_all(first_directory).unwrap();
    fs::remove_dir_all(second_directory).unwrap();
}

#[test]
fn excluded_video_only_first_source_does_not_block_audio_extract_defaults() {
    let (first_directory, first_path) = temp_file("batch-video-only-first");
    let (second_directory, second_path) = temp_file("batch-audio-valid-second");
    let mut first_media = media(&first_path);
    first_media
        .streams
        .retain(|stream| stream.kind != StreamKind::Audio);
    first_media.audio_streams.clear();
    let second_media = media(&second_path);
    let mut app = app();
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::AudioExtract,
        defaults: Some(BatchDefaults::AudioExtract {
            format: AudioExtractFormat::Original,
        }),
        items: vec![
            BatchItem {
                path: first_path,
                included: false,
                source: ready_batch_source(first_media),
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            },
            BatchItem {
                path: second_path.clone(),
                included: true,
                source: ready_batch_source(second_media),
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            },
        ],
        error: None,
        append_error: None,
        append_error_details_open: false,
    });
    app.core.show_batch_setup();
    refresh_batch_items(app.batch_setup.as_mut().unwrap(), None);

    assert!(app.begin_batch_edit(BatchEditTarget::Defaults));
    app.batch_edit
        .as_mut()
        .unwrap()
        .audio_extract
        .set_format(AudioExtractFormat::Flac);
    assert!(app.save_batch_edit());

    let batch = app.batch_setup.as_ref().unwrap();
    assert_eq!(batch.items[0].readiness, BatchReadiness::Incompatible);
    assert_eq!(batch.items[0].reason.as_deref(), Some("No audio tracks."));
    assert_eq!(batch.items[1].readiness, BatchReadiness::Ready);
    assert!(matches!(
        batch.items[1].operation.as_ref(),
        Some(QueuedOperation::AudioExtract(request))
            if request.input == second_path && request.format == AudioExtractFormat::Flac
    ));
    assert_no_batch_execution(&app);
    fs::remove_dir_all(first_directory).unwrap();
    fs::remove_dir_all(second_directory).unwrap();
}

#[test]
fn batch_override_cancel_is_non_mutating_and_save_updates_only_the_item() {
    let (directory, path) = temp_file("batch-override");
    let source = media(&path);
    let mut app = app();
    let selection = crate::remux::draft::RemuxDraft::new(&source);
    let original = QueuedOperation::Remux(selection.request(&source));
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::Remux,
        defaults: Some(BatchDefaults::Remux {
            container: MediaContainer::Matroska,
        }),
        items: vec![BatchItem {
            path: path.clone(),
            included: true,
            source: ready_batch_source(source),
            operation: Some(original.clone()),
            readiness: BatchReadiness::Ready,
            reason: None,
            overridden: false,
        }],
        error: None,
        append_error: None,
        append_error_details_open: false,
    });
    app.core.show_batch_setup();

    assert!(app.begin_batch_edit(BatchEditTarget::Item(0)));
    app.batch_edit
        .as_mut()
        .unwrap()
        .remux
        .set_custom_output_path(directory.join("cancelled.mkv"));
    assert!(app.finish_batch_edit());
    assert_eq!(
        app.batch_setup.as_ref().unwrap().items[0].operation,
        Some(original.clone())
    );

    assert!(app.begin_batch_edit(BatchEditTarget::Item(0)));
    let saved_output = directory.join("saved.mkv");
    app.batch_edit
        .as_mut()
        .unwrap()
        .remux
        .set_custom_output_path(saved_output.clone());
    assert!(app.save_batch_edit());
    assert_eq!(
        app.batch_setup.as_ref().unwrap().items[0]
            .operation
            .as_ref()
            .map(QueuedOperation::output),
        Some(saved_output.as_path())
    );
    let saved_operation = app.batch_setup.as_ref().unwrap().items[0].operation.clone();
    assert!(app.begin_batch_edit(BatchEditTarget::Defaults));
    app.batch_edit
        .as_mut()
        .unwrap()
        .remux
        .set_container(MediaContainer::Mp4);
    assert!(app.save_batch_edit());
    let item = &app.batch_setup.as_ref().unwrap().items[0];
    assert!(item.overridden);
    assert_eq!(item.operation, saved_operation);
    assert_no_batch_execution(&app);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opening_upscale_source_renders_ready_surface() {
    let (directory, path) = temp_file("upscale-ready-render");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| {
        show_tool(&mut app, ui, Operation::VideoUpscale);
    });
    output.textures_delta.clear();

    assert!(output_contains_text(&output, "Upscale to"));
    assert!(output_contains_text(&output, "Output"));
    assert!(output_contains_text(&output, "H.264"));
    assert!(output_contains_text(&output, "Browse"));
    assert!(!output_contains_text(&output, "Video codec"));
    assert!(!output_contains_text(&output, "Actual output"));
    assert!(!output_contains_text(&output, "← Back"));
    assert!(output_contains_text(&output, "Change file"));
    assert!(output_contains_text(&output, "Close source"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opening_remux_source_renders_ready_surface() {
    let (directory, path) = temp_file("remux-ready-render");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| {
        show_tool(&mut app, ui, Operation::Remux);
    });
    output.textures_delta.clear();

    for text in [
        "recording.mkv",
        "1920×1080",
        "MKV",
        "Output",
        "Browse",
        "Remux",
    ] {
        assert!(output_contains_text(&output, text), "missing {text:?}");
    }
    assert!(!output_contains_text(&output, "Container"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opening_audio_extract_source_renders_ready_surface() {
    let (directory, path) = temp_file("audio-extract-ready-render");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::AudioExtract);
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| {
        show_tool(&mut app, ui, Operation::AudioExtract);
    });
    output.textures_delta.clear();
    for text in [
        "recording.mkv",
        "1920×1080",
        "Audio track",
        "Desktop Audio",
        "stream 1",
        "Original (copy)",
        "no re-encode · MKA",
        "Output",
        "Browse",
        "Extract",
    ] {
        assert!(output_contains_text(&output, text), "missing {text:?}");
    }
    assert!(!output_contains_text(&output, "Format"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opening_audio_mix_source_renders_ready_surface() {
    let (directory, path) = temp_file("audio-ready-render");
    let mut app = app();
    app.select_tool(Operation::AudioMix);
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| {
        show_tool(&mut app, ui, Operation::AudioMix);
    });
    output.textures_delta.clear();

    assert!(output_contains_text(&output, "Audio tracks"));
    assert!(output_contains_text(&output, "Loudness normalization"));
    assert!(output_contains_text(&output, "Output"));
    assert!(output_contains_text(&output, "MP4"));
    assert!(output_contains_text(&output, "Browse"));
    assert!(!output_contains_text(&output, "Integrated loudness"));
    assert!(output_contains_text(&output, "Mix selected tracks"));
    assert!(!output_contains_text(&output, "← Back"));
    assert!(output_contains_text(&output, "Change file"));
    let audio_tracks = output_text_rect(&output, "Audio tracks");
    let select_all = output_text_rect(&output, "Select all");
    let clear = output_text_rect(&output, "Clear");
    let selected = output_text_rect(&output, "1 selected");
    let reset_gains = output_text_rect(&output, "Reset gains");
    let loudness = output_text_rect(&output, "Loudness normalization");
    assert!((audio_tracks.center().y - select_all.center().y).abs() <= 1.0);
    assert!((audio_tracks.center().y - clear.center().y).abs() <= 1.0);
    assert!((selected.center().y - reset_gains.center().y).abs() <= 1.0);
    assert!(selected.top() > audio_tracks.bottom());
    assert!(loudness.top() > selected.bottom());

    app.state
        .audio_mix
        .as_mut()
        .unwrap()
        .selection
        .set_normalization_enabled(true);
    let mut output = context.run_ui(Default::default(), |ui| {
        show_tool(&mut app, ui, Operation::AudioMix);
    });
    output.textures_delta.clear();
    for label in [
        "Integrated loudness",
        "Maximum true peak",
        "Loudness range",
        "Reset normalization targets",
    ] {
        assert!(output_contains_text(&output, label), "missing {label}");
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn shared_workspace_chrome_is_identical_across_all_ready_operations() {
    let (directory, path) = temp_file("shared-workspace-chrome");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    for operation in Operation::ALL {
        let mut output = context.run_ui(Default::default(), |ui| {
            show_tool(&mut app, ui, operation);
        });
        output.textures_delta.clear();

        assert_eq!(output_text_count(&output, "recording.mkv"), 1);
        for label in [
            "1920×1080",
            "Change file",
            "Close source",
            "Audio Mix",
            "Upscale",
            "Remux",
            "Audio Extract",
            "Frame Capture",
        ] {
            assert!(
                output_contains_text(&output, label),
                "{operation:?} missing workspace label {label:?}"
            );
        }
        assert!(!output_contains_text(&output, "← Back"));
    }

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn standalone_operation_feedback_does_not_render_workspace_components() {
    let (directory, path) = temp_file("standalone-operation-feedback");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    app.state.frame_capture.as_mut().unwrap().start_error =
        Some(MediaError::invalid_request("simulated start failure"));
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
    output.textures_delta.clear();

    assert!(output_contains_text(
        &output,
        "Frame capture could not start"
    ));
    for workspace_label in [
        "Change file",
        "Close source",
        "Audio Mix",
        "Upscale",
        "Remux",
        "Audio Extract",
        "Frame Capture",
    ] {
        assert!(!output_contains_text(&output, workspace_label));
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn every_ready_operation_keeps_its_primary_action_bottom_owned_at_all_heights() {
    let (directory, path) = temp_file("shared-operation-shell");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    let action_height = theme::PRIMARY_CONTROL_HEIGHT
        + f32::from(layout::ACTION_BAR_VERTICAL_PADDING) * 2.0
        + theme::SEPARATOR_WIDTH;

    for (operation, label) in [
        (Operation::AudioMix, "Mix selected tracks"),
        (Operation::VideoUpscale, "Upscale"),
        (Operation::Remux, "Remux"),
        (Operation::AudioExtract, "Extract"),
        (Operation::FrameCapture, "Capture frame"),
    ] {
        for (width, height) in [
            (400.0, 260.0),
            (620.0, 379.0),
            (1_200.0, 700.0),
            (620.0, 900.0),
        ] {
            let mut viewport = egui::Rect::NOTHING;
            let mut output = context.run_ui(Default::default(), |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                viewport = rect;
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                );
                child.set_clip_rect(rect);
                show_tool(&mut app, &mut child, operation);
            });
            output.textures_delta.clear();
            let action = output_bottom_text_rect(&output, label);
            let source = output_text_rect(&output, "recording.mkv");

            assert!(
                (action.center().y - (viewport.bottom() - action_height * 0.5)).abs() <= 1.0,
                "{operation:?} at {width}x{height}: action={action:?}, viewport={viewport:?}"
            );
            assert!(source.top() >= viewport.top() + f32::from(layout::CONTENT_PADDING));
            assert!(source.bottom() < action.top());
        }
    }

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn upscale_ready_selection_survives_same_source_navigation() {
    let (directory, path) = temp_file("selection-preserve");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);
    let ready = app
        .state
        .upscale
        .as_mut()
        .expect("upscale session should exist");
    ready
        .selection
        .set_target(&ready.media, UpscaleTarget::Uhd8k);
    ready.selection.set_codec(VideoCodec::H265);

    app.select_tool(Operation::VideoUpscale);

    let ready = app
        .state
        .upscale
        .as_ref()
        .expect("upscale session should persist");
    assert_eq!(ready.selection.target(), UpscaleTarget::Uhd8k);
    assert_eq!(ready.selection.codec(), VideoCodec::H265);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn upscale_intent_reaches_executor() {
    let (directory, path) = temp_file("execute");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);

    let error = app
        .handle_upscale_action(UpscaleSelectionAction::UpscaleRequested)
        .expect_err("missing test FFmpeg should prove the execution path was attempted");

    assert!(matches!(
        error,
        crate::upscale::execution::UpscaleStartError::Process(
            crate::media::process::MediaProcessStartError::ProcessLaunchFailed(
                std::io::ErrorKind::NotFound
            )
        )
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn every_ready_request_owner_builds_an_immutable_queue_operation() {
    let (directory, path) = temp_file("queue-requests");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));

    for operation in [
        Operation::AudioMix,
        Operation::VideoUpscale,
        Operation::Remux,
        Operation::AudioExtract,
        Operation::FrameCapture,
    ] {
        app.select_tool(operation);
        assert!(app.enqueue_current_operation(operation).unwrap().is_some());
    }

    assert_eq!(app.core.media_queue.jobs().len(), 5);
    assert!(matches!(
        app.core.media_queue.jobs()[0].operation(),
        QueuedOperation::AudioMix(_)
    ));
    assert!(matches!(
        app.core.media_queue.jobs()[1].operation(),
        QueuedOperation::Upscale(_)
    ));
    assert!(matches!(
        app.core.media_queue.jobs()[2].operation(),
        QueuedOperation::Remux(_)
    ));
    assert!(matches!(
        app.core.media_queue.jobs()[3].operation(),
        QueuedOperation::AudioExtract(_)
    ));
    assert!(matches!(
        app.core.media_queue.jobs()[4].operation(),
        QueuedOperation::FrameCapture(_)
    ));
    assert!(
        app.core
            .media_queue
            .jobs()
            .iter()
            .all(|job| matches!(job.state(), QueueJobState::Waiting))
    );
    assert!(app.execution_arbitration().queue_start_allowed());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn every_ready_enqueue_action_remains_in_the_workspace_without_launching() {
    let (directory, path) = temp_file("queue-actions");
    let mut app = app();

    for operation in [
        Operation::AudioMix,
        Operation::VideoUpscale,
        Operation::Remux,
        Operation::AudioExtract,
        Operation::FrameCapture,
    ] {
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(operation);
        let previous_count = app.core.media_queue.jobs().len();

        match operation {
            Operation::AudioMix => app
                .handle_audio_mix_action(AudioMixSelectionAction::QueueRequested)
                .unwrap(),
            Operation::VideoUpscale => app
                .handle_upscale_action(UpscaleSelectionAction::QueueRequested)
                .unwrap(),
            Operation::Remux => app
                .handle_remux_action(RemuxSelectionAction::QueueRequested)
                .unwrap(),
            Operation::AudioExtract => app
                .handle_audio_extract_action(AudioExtractSelectionAction::QueueRequested)
                .unwrap(),
            Operation::FrameCapture => app
                .handle_frame_capture_action(FrameCaptureSelectionAction::QueueRequested)
                .unwrap(),
        }

        assert_eq!(app.core.media_queue.jobs().len(), previous_count + 1);
        assert_eq!(
            app.core.navigation(),
            NavigationState::SourceWorkspace {
                active_operation: operation
            }
        );
        assert!(app.execution_arbitration().queue_start_allowed());
        assert!(!app.core.media_queue.is_running());
    }

    assert!(
        app.core
            .media_queue
            .jobs()
            .iter()
            .all(|job| matches!(job.state(), QueueJobState::Waiting))
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_ready_request_cannot_be_enqueued_by_direct_intent() {
    let (directory, path) = temp_file("invalid-queue-action");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.state.audio_mix.as_mut().unwrap().selection.clear();

    app.handle_audio_mix_action(AudioMixSelectionAction::QueueRequested)
        .unwrap();

    assert!(app.core.media_queue.jobs().is_empty());
    assert_eq!(app.core.queue_confirmation, None);
    assert!(matches!(
        app.core.navigation(),
        NavigationState::SourceWorkspace {
            active_operation: Operation::AudioMix
        }
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn duplicate_queued_destination_is_truthful_operation_local_feedback() {
    let (directory, path) = temp_file("queue-confirmation");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));

    app.handle_audio_mix_action(AudioMixSelectionAction::QueueRequested)
        .unwrap();

    assert_eq!(app.core.media_queue.pending_count(), 1);
    let output_path = app.core.media_queue.jobs()[0]
        .operation()
        .output()
        .to_path_buf();
    assert!(!output_path.exists());
    assert_eq!(app.core.queue_confirmation, Some(Operation::AudioMix));
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
    assert!(output_contains_text(&output, "Added to queue"));
    output.textures_delta.clear();

    app.handle_audio_mix_action(AudioMixSelectionAction::QueueRequested)
        .unwrap();
    assert_eq!(app.core.media_queue.pending_count(), 1);
    assert!(!output_path.exists());
    assert_eq!(app.core.queue_confirmation, None);
    assert!(app.core.source.error().is_none());
    assert!(matches!(
        app.core.operation_action_error,
        Some((Operation::AudioMix, ref error))
            if error.kind() == MediaErrorKind::QueueDestinationReserved
                && error.diagnostic().is_some_and(|diagnostic| diagnostic.contains("queue job 1"))
    ));

    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
    assert!(output_contains_text(
        &output,
        "Another queued job already targets this output."
    ));
    output.textures_delta.clear();

    let error = app
        .core
        .operation_action_error
        .as_ref()
        .map(|(_, error)| error.clone())
        .unwrap();
    let mut details_open = true;
    let mut output = context.run_ui(Default::default(), |ui| {
        media_error::show_inline(ui, &error, &mut details_open)
    });
    assert!(output_contains_text(&output, "queue job 1"));
    output.textures_delta.clear();

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn genuine_existing_output_keeps_filesystem_output_exists_feedback() {
    let (directory, path) = temp_file("queue-existing-output");
    let existing_output = directory.join("already-there.mp4");
    fs::write(&existing_output, b"existing output").unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.state
        .audio_mix
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(existing_output);

    app.enqueue_from_ready_surface(Operation::AudioMix);

    assert!(app.core.media_queue.jobs().is_empty());
    assert!(app.core.source.error().is_none());
    assert!(matches!(
        app.core.operation_action_error,
        Some((Operation::AudioMix, ref error))
            if error.kind() == MediaErrorKind::OutputExists
                && error.guidance() == Some("Choose another output path or move the existing file.")
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn batch_append_ignores_duplicates_without_mutating_existing_configuration() {
    let (directory, path) = temp_file("batch-append-duplicate");
    let source = media(&path);
    let operation =
        QueuedOperation::Remux(crate::remux::draft::RemuxDraft::new(&source).request(&source));
    let mut app = app();
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::Remux,
        defaults: Some(BatchDefaults::Remux {
            container: MediaContainer::Matroska,
        }),
        items: vec![BatchItem {
            path: path.clone(),
            included: true,
            source: ready_batch_source(source),
            operation: Some(operation.clone()),
            readiness: BatchReadiness::Ready,
            reason: None,
            overridden: true,
        }],
        error: None,
        append_error: None,
        append_error_details_open: false,
    });

    assert!(!app.append_batch_paths(vec![path.clone(), path.clone()]));
    let batch = app.batch_setup.as_ref().unwrap();
    assert_eq!(batch.items.len(), 1);
    assert_eq!(batch.items[0].operation, Some(operation));
    assert!(batch.items[0].overridden);
    assert_eq!(
        batch.error.as_deref(),
        Some("Ignored 2 files that are already in this batch.")
    );
    assert!(app.batch_inspection.is_none());
    assert!(app.core.media_queue.jobs().is_empty());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn relevant_workspace_actions_clear_enqueue_confirmation() {
    let (directory, path) = temp_file("queue-confirmation-clear");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.enqueue_from_ready_surface(Operation::AudioMix);
    assert_eq!(app.core.queue_confirmation, Some(Operation::AudioMix));

    app.select_tool(Operation::Remux);
    assert_eq!(app.core.queue_confirmation, None);

    app.select_tool(Operation::AudioMix);
    app.state
        .audio_mix
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(directory.join("another.mkv"));
    app.enqueue_from_ready_surface(Operation::AudioMix);
    let job_id = app.core.media_queue.jobs()[1].id();
    assert_eq!(app.core.queue_confirmation, Some(Operation::AudioMix));
    assert!(app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
    assert_eq!(app.core.queue_confirmation, None);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn normal_queue_and_batch_drafts_round_trip_each_operation_without_mutating_other_editors() {
    use super::state::OperationEditorState;
    use crate::upscale::model::{CpuQuality, VideoEncoderSelection};

    fn set_editor_stem(edit: &mut OperationEditorState, stem: &OsStr) {
        let stem = stem.to_os_string();
        match edit.active_operation {
            Operation::AudioMix => edit.audio_mix.set_output_filename_stem(stem),
            Operation::VideoUpscale => edit.upscale.set_output_filename_stem(stem),
            Operation::Remux => edit.remux.set_output_filename_stem(stem),
            Operation::AudioExtract => edit.audio_extract.set_output_filename_stem(stem),
            Operation::FrameCapture => edit.frame_capture.set_output_filename_stem(stem),
        }
    }

    let (directory, path) = temp_file("draft-round-trip");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let mut job_ids = Vec::new();
    for operation in Operation::ALL {
        app.select_tool(operation);
        match operation {
            Operation::AudioMix => {
                let session = app.state.audio_mix.as_mut().unwrap();
                session
                    .selection
                    .set_track_gain(&session.media, 1, TrackGain::from_tenths_db(-35));
                session.selection.set_normalization_enabled(true);
                session
                    .selection
                    .set_normalization_targets(LoudnessTargets::from_tenths(-180, -20, 100));
                session
                    .selection
                    .set_container(&session.media, OutputContainer::Mkv);
                session
                    .selection
                    .set_output_filename_stem("custom-mix".into());
            }
            Operation::VideoUpscale => {
                let session = app.state.upscale.as_mut().unwrap();
                session.selection.set_target(
                    &session.media,
                    UpscaleTarget::Custom {
                        max_width: 2560,
                        max_height: 1440,
                    },
                );
                session.selection.set_codec(VideoCodec::H265);
                session
                    .selection
                    .set_encoder_selection(VideoEncoderSelection::Cpu);
                session
                    .selection
                    .set_cpu_quality(VideoCodec::H265, CpuQuality::new(23).unwrap());
                session
                    .selection
                    .set_output_filename_stem("custom-upscale".into());
            }
            Operation::Remux => {
                let session = app.state.remux.as_mut().unwrap();
                session.selection.set_container(MediaContainer::Mp4);
                session
                    .selection
                    .set_output_filename_stem("custom-remux".into());
            }
            Operation::AudioExtract => {
                let session = app.state.audio_extract.as_mut().unwrap();
                session.selection.set_format(AudioExtractFormat::Flac);
                session
                    .selection
                    .set_output_filename_stem("custom-audio".into());
            }
            Operation::FrameCapture => {
                let session = app.state.frame_capture.as_mut().unwrap();
                session.editor.selection.set_position(
                    &session.media,
                    FrameTimestamp::from_milliseconds(2250).into(),
                );
                session.selection.set_format(FrameCaptureFormat::Jpeg);
                session
                    .selection
                    .set_output_filename_stem("custom-frame".into());
            }
        }
        job_ids.push(app.enqueue_current_operation(operation).unwrap().unwrap());
    }
    app.enter_queue();
    let originals = app
        .core
        .media_queue
        .jobs()
        .iter()
        .map(|job| job.operation().clone())
        .collect::<Vec<_>>();
    for (job_id, original) in job_ids.into_iter().zip(&originals) {
        assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
        let stem = original.output().file_stem().unwrap();
        set_editor_stem(&mut app.queue_edit.as_mut().unwrap().editor, OsStr::new(""));
        assert!(!app.save_queue_edit());
        assert_eq!(
            app.core
                .media_queue
                .jobs()
                .iter()
                .find(|job| job.id() == job_id)
                .unwrap()
                .operation(),
            original
        );
        set_editor_stem(&mut app.queue_edit.as_mut().unwrap().editor, stem);
        assert!(app.save_queue_edit());
        assert_eq!(
            app.core
                .media_queue
                .jobs()
                .iter()
                .find(|job| job.id() == job_id)
                .unwrap()
                .operation(),
            original
        );

        app.batch_setup = Some(BatchSetupState {
            origin: BatchOrigin::Queue {
                origin: QueueOrigin::SourceWorkspace {
                    active_operation: Operation::FrameCapture,
                },
            },
            operation: Operation::from(original),
            defaults: None,
            items: vec![BatchItem {
                path: path.clone(),
                included: true,
                source: ready_batch_source(media(&path)),
                operation: Some(original.clone()),
                readiness: BatchReadiness::Ready,
                reason: None,
                overridden: false,
            }],
            error: None,
            append_error: None,
            append_error_details_open: false,
        });
        app.core.show_batch_setup();
        assert!(app.begin_batch_edit(BatchEditTarget::Item(0)));
        set_editor_stem(&mut app.batch_edit.as_mut().unwrap().editor, OsStr::new(""));
        assert!(!app.save_batch_edit());
        assert_eq!(
            app.batch_setup.as_ref().unwrap().items[0]
                .operation
                .as_ref(),
            Some(original)
        );
        set_editor_stem(&mut app.batch_edit.as_mut().unwrap().editor, stem);
        assert!(app.save_batch_edit());
        assert_eq!(
            app.batch_setup.as_ref().unwrap().items[0]
                .operation
                .as_ref(),
            Some(original)
        );
        assert!(app.finish_batch_setup());
    }
    // Independent editor changes never rewrite normal source drafts.
    for original in &originals {
        let request = match original {
            QueuedOperation::AudioMix(_) => {
                let s = app.state.audio_mix.as_ref().unwrap();
                QueuedOperation::AudioMix(s.selection.request(&s.media))
            }
            QueuedOperation::Upscale(_) => {
                let s = app.state.upscale.as_ref().unwrap();
                QueuedOperation::Upscale(
                    s.selection
                        .request_for_capabilities(
                            &s.media,
                            app.core.discovery.upscale_capabilities().unwrap(),
                        )
                        .unwrap(),
                )
            }
            QueuedOperation::Remux(_) => {
                let s = app.state.remux.as_ref().unwrap();
                QueuedOperation::Remux(s.selection.request(&s.media))
            }
            QueuedOperation::AudioExtract(_) => {
                let s = app.state.audio_extract.as_ref().unwrap();
                QueuedOperation::AudioExtract(s.selection.request(&s.media).unwrap())
            }
            QueuedOperation::FrameCapture(_) => {
                let s = app.state.frame_capture.as_ref().unwrap();
                QueuedOperation::FrameCapture(s.selection.request(&s.media).unwrap())
            }
        };
        assert_eq!(&request, original);
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_edit_reuses_workspace_without_source_or_execution_actions() {
    let (directory, path) = temp_file("queue-edit-surface");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let job_id = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    app.enter_queue();
    assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));

    for label in [
        "Editing queued item",
        "Audio Mix",
        "Upscale",
        "Remux",
        "Audio Extract",
        "Frame Capture",
        "Save changes",
        "Cancel edit",
    ] {
        assert!(output_contains_text(&output, label), "missing {label:?}");
    }
    for label in ["Change file", "Close source", "Add to queue"] {
        assert!(
            !output_contains_text(&output, label),
            "unexpected {label:?}"
        );
    }
    output.textures_delta.clear();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stopped_queue_surface_exposes_edit_remove_and_bounded_reorder_controls() {
    let (directory, path) = temp_file("queue-controls-surface");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    app.enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    app.enter_queue();
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));

    for label in ["Edit", "Remove", "↑", "↓", "Start queue", "Back"] {
        assert!(output_contains_text(&output, label), "missing {label:?}");
    }
    output.textures_delta.clear();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_edit_failed_replacement_keeps_original_job_and_editor() {
    let (directory, path) = temp_file("queue-edit-failure");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let first = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(directory.join("second.mkv"));
    let second = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    let original = app.core.media_queue.jobs()[0].operation().clone();
    let duplicate_output = app.core.media_queue.jobs()[1]
        .operation()
        .output()
        .to_path_buf();
    app.enter_queue();
    assert!(app.handle_queue_action(QueueAction::Edit(first)));
    app.queue_edit
        .as_mut()
        .unwrap()
        .remux
        .set_custom_output_path(duplicate_output);

    assert!(!app.save_queue_edit());

    assert_eq!(app.core.media_queue.jobs()[0].id(), first);
    assert_eq!(app.core.media_queue.jobs()[0].operation(), &original);
    assert_eq!(app.core.media_queue.jobs()[1].id(), second);
    assert!(app.queue_edit.as_ref().unwrap().error.is_some());
    assert!(matches!(
        app.core.navigation(),
        NavigationState::QueueEdit { .. }
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_reorder_actions_move_only_the_requested_waiting_item() {
    let (directory, path) = temp_file("queue-reorder-actions");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let first = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(directory.join("later.mkv"));
    let second = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();

    assert!(app.handle_queue_action(QueueAction::MoveEarlier(second)));
    assert_eq!(app.core.media_queue.jobs()[0].id(), second);
    assert_eq!(app.core.media_queue.jobs()[1].id(), first);
    assert!(app.handle_queue_action(QueueAction::MoveLater(second)));
    assert_eq!(app.core.media_queue.jobs()[0].id(), first);
    assert_eq!(app.core.media_queue.jobs()[1].id(), second);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn completed_queue_output_resolves_its_parent_folder_only_for_success() {
    let completed = QueueJobState::Completed {
        output: PathBuf::from("exports/output.mkv"),
    };
    assert_eq!(
        completed_output_folder(&completed),
        Some(Path::new("exports"))
    );
    assert_eq!(completed_output_folder(&QueueJobState::Waiting), None);
    assert_eq!(
        completed_output_folder(&QueueJobState::Failed(QueueJobFailure::SourceChanged)),
        None
    );
}

#[test]
fn queue_remove_and_clear_actions_respect_job_state() {
    let (directory, path) = temp_file("queue-row-actions");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let waiting = app
        .enqueue_current_operation(Operation::AudioMix)
        .unwrap()
        .unwrap();

    assert!(app.handle_queue_action(QueueAction::Remove(waiting)));
    assert!(app.core.media_queue.jobs().is_empty());

    let terminal = app
        .enqueue_current_operation(Operation::AudioMix)
        .unwrap()
        .unwrap();
    assert!(app.handle_queue_action(QueueAction::CancelQueue));
    assert!(!app.handle_queue_action(QueueAction::Remove(terminal)));
    assert!(app.handle_queue_action(QueueAction::ClearFinished));
    assert!(app.core.media_queue.jobs().is_empty());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_terminal_failure_does_not_replace_the_ready_source_session() {
    let (directory, path) = temp_file("queue-session");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let session_id = app.state.remux.as_ref().unwrap().session_id;
    let job_id = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();

    assert!(!app.start_media_queue());

    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(app.state.remux.as_ref().unwrap().session_id, session_id);
    assert_eq!(app.core.media_queue.jobs()[0].id(), job_id);
    assert!(matches!(
        app.core.media_queue.jobs()[0].state(),
        QueueJobState::Failed(QueueJobFailure::ProcessStart(_))
    ));
    assert!(app.execution_arbitration().queue_start_allowed());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_process_events_are_tagged_and_filtered_by_job_id() {
    let (directory, path) = temp_file("queue-events");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let first = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(directory.join("second-remux.mkv"));
    let second = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    let wake = queue_execution_wake(first, sender, MediaProcessWake::default());

    wake.notify(MediaProcessEvent::Progress);
    let event = receiver.recv_timeout(Duration::from_secs(10)).unwrap();

    assert_eq!(
        event,
        QueueExecutionEvent {
            job_id: first,
            process_event: MediaProcessEvent::Progress,
        }
    );
    assert!(queue_event_matches_active(Some(first), event));
    assert!(!queue_event_matches_active(Some(second), event));
    assert!(!queue_event_matches_active(None, event));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn remux_intent_reaches_executor() {
    let (directory, path) = temp_file("remux-execute");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);

    let error = app
        .handle_remux_action(RemuxSelectionAction::RemuxRequested)
        .expect_err("missing test FFmpeg should prove the execution path was attempted");

    assert!(matches!(
        error,
        crate::remux::execution::RemuxStartError::Process(
            crate::media::process::MediaProcessStartError::ProcessLaunchFailed(
                std::io::ErrorKind::NotFound
            )
        )
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn audio_extract_intent_reaches_executor() {
    let (directory, path) = temp_file("audio-extract-execute");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::AudioExtract);

    let error = app
        .handle_audio_extract_action(AudioExtractSelectionAction::ExtractRequested)
        .expect_err("missing test FFmpeg should prove the execution path was attempted");
    assert!(matches!(
        error,
        crate::audio_extract::execution::AudioExtractStartError::Process(
            crate::media::process::MediaProcessStartError::ProcessLaunchFailed(
                std::io::ErrorKind::NotFound
            )
        )
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn each_operation_routes_only_non_idle_execution_to_lifecycle_ui() {
    let states = [
        (AudioMixExecutionState::Idle, false),
        (AudioMixExecutionState::Running, true),
        (
            AudioMixExecutionState::Completed {
                output: PathBuf::from("recording.mp4"),
            },
            true,
        ),
        (
            AudioMixExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            }),
            true,
        ),
        (AudioMixExecutionState::Cancelled, true),
    ];
    for (state, visible) in states {
        for (name, route) in [
            (
                "audio mix",
                shows_audio_mix_lifecycle as fn(&AudioMixExecutionState) -> bool,
            ),
            ("upscale", shows_upscale_lifecycle),
            ("remux", shows_remux_lifecycle),
            ("audio extract", shows_audio_extract_lifecycle),
            ("frame capture", shows_frame_capture_lifecycle),
        ] {
            assert_eq!(route(&state), visible, "{name}: {state:?}");
        }
    }
}

#[test]
fn audio_mix_state_survives_operation_switching() {
    let (directory, path) = temp_file("audio-back");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let session = app
        .state
        .audio_mix
        .as_mut()
        .expect("audio mix session should exist");
    session.selection.toggle_stream(&session.media, 1);
    assert!(
        session
            .selection
            .set_track_gain(&session.media, 1, TrackGain::from_tenths_db(-55),)
    );
    session
        .selection
        .set_container(&session.media, OutputContainer::Mkv);
    let custom_output = directory.join("custom-mix.mkv");
    session
        .selection
        .set_custom_output_path(custom_output.clone());
    let normalization_targets = LoudnessTargets::from_tenths(-205, -8, 145);
    assert!(
        session
            .selection
            .set_normalization_targets(normalization_targets)
    );
    session.selection.set_normalization_enabled(true);

    app.select_tool(Operation::VideoUpscale);
    app.select_tool(Operation::AudioMix);

    let session = app
        .state
        .audio_mix
        .as_ref()
        .expect("same-source Audio Mix session should be restored");
    assert_workspace(&app.core, Operation::AudioMix);
    assert_eq!(session.selection.selected_count(), 0);
    assert_eq!(
        session.selection.track_gain(1),
        Some(TrackGain::from_tenths_db(-55))
    );
    assert_eq!(session.selection.container(), OutputContainer::Mkv);
    assert_eq!(session.selection.output_path(), custom_output);
    assert!(session.selection.output_is_customized());
    assert!(session.selection.normalization_enabled());
    assert_eq!(
        session.selection.normalization_targets(),
        normalization_targets
    );
    assert_eq!(
        session.selection.request(&session.media).normalization,
        LoudnessNormalization::Enabled(normalization_targets)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn upscale_state_survives_operation_switching() {
    let (directory, path) = temp_file("upscale-back");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);
    let session = app
        .state
        .upscale
        .as_mut()
        .expect("upscale session should exist");
    session.selection.select_custom_target(&session.media);
    session
        .selection
        .set_custom_boundary(&session.media, 3000, 2000);
    session.selection.set_codec(VideoCodec::H265);
    let custom_output = directory.join("custom-upscale.mp4");
    session
        .selection
        .set_custom_output_path(custom_output.clone());

    app.select_tool(Operation::AudioMix);
    app.select_tool(Operation::VideoUpscale);

    let session = app
        .state
        .upscale
        .as_ref()
        .expect("same-source Upscale session should be restored");
    assert_workspace(&app.core, Operation::VideoUpscale);
    assert_eq!(
        session.selection.target(),
        UpscaleTarget::Custom {
            max_width: 3000,
            max_height: 2000,
        }
    );
    assert_eq!(session.selection.custom_boundary(), Some((3000, 2000)));
    assert_eq!(session.selection.codec(), VideoCodec::H265);
    assert_eq!(session.selection.output_path(), custom_output);
    assert!(session.selection.output_is_customized());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn remux_state_survives_operation_switching() {
    let (directory, path) = temp_file("remux-back");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let session = app
        .state
        .remux
        .as_mut()
        .expect("remux session should exist");
    session.selection.set_container(MediaContainer::Mp4);
    let custom_output = directory.join("custom-remux.mp4");
    session
        .selection
        .set_custom_output_path(custom_output.clone());

    app.select_tool(Operation::AudioMix);
    app.select_tool(Operation::Remux);

    let session = app
        .state
        .remux
        .as_ref()
        .expect("same-source Remux session should be restored");
    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(session.selection.container(), MediaContainer::Mp4);
    assert_eq!(session.selection.output_path(), custom_output);
    assert!(session.selection.output_is_customized());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn audio_extract_state_survives_operation_switching() {
    let (directory, path) = temp_file("audio-extract-back");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::AudioExtract);
    let session = app
        .state
        .audio_extract
        .as_mut()
        .expect("audio extract session should exist");
    session.selection.set_format(AudioExtractFormat::Flac);
    let custom_output = directory.join("dialogue.flac");
    session
        .selection
        .set_custom_output_path(custom_output.clone());

    app.select_tool(Operation::AudioMix);
    app.select_tool(Operation::AudioExtract);
    let session = app
        .state
        .audio_extract
        .as_ref()
        .expect("same-source Audio Extract session should be restored");
    assert_workspace(&app.core, Operation::AudioExtract);
    assert_eq!(session.selection.selected_stream_index(), Some(1));
    assert_eq!(session.selection.format(), AudioExtractFormat::Flac);
    assert_eq!(session.selection.output_path(), custom_output);
    assert!(session.selection.output_is_customized());
    fs::remove_dir_all(directory).unwrap();
}

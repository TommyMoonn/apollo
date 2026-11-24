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
    FrameCaptureThumbnailLoad, FrameCaptureToolState, FramePreviewDecode, QueueExecutionEvent,
    frame_navigation_availability,
};
use super::workflows::{
    ToolExecutionStates, audio_extract_output_dialog_request, audio_mix_output_dialog_request,
    completed_output_folder, frame_capture_output_dialog_request, queue_event_matches_active,
    queue_execution_wake, refresh_batch_items, remux_output_dialog_request,
    settle_output_file_dialog, shows_audio_extract_lifecycle, shows_audio_mix_lifecycle,
    shows_frame_capture_lifecycle, shows_remux_lifecycle, shows_upscale_lifecycle,
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

use crate::app::{QueueOrigin, file_intake_enabled};
use crate::file_dialog::{
    DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogOwner, FileDialogRequest,
    FileDialogStartError, OutputFormat,
};
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
        QueueAction, SourceAction,
        audio_extract::AudioExtractSelectionAction,
        audio_mix::AudioMixSelectionAction,
        batch::BatchReadiness,
        file_intake::FileIntakeAction,
        fonts,
        frame_capture::{FrameCaptureSelectionAction, FrameCaptureSelectionState},
        frame_capture_progress::FrameCaptureThumbnailState,
        layout, media_error,
        remux::RemuxSelectionAction,
        theme,
        upscale::UpscaleSelectionAction,
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
    while wake_rx.try_recv().is_ok() {}
    app.tools_error_details_open = true;
    release_tx.send(()).unwrap();
    wake_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("capability completion wakes egui");
    app.poll_background_work();
    assert!(!app.core.discovery.is_pending());
    assert!(!app.tools_error_details_open);
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
fn frame_thumbnail_decode_runs_on_the_dedicated_worker() {
    let (thread_sender, thread_receiver) = mpsc::sync_channel(1);
    let (wake_tx, wake_rx) = mpsc::channel();
    let wake = move || {
        let _ = wake_tx.send(());
    };
    let load = FrameCaptureThumbnailLoad::spawn_with(7, 11, wake, move || {
        thread_sender.send(std::thread::current().id()).unwrap();
        Ok(thumbnail())
    })
    .expect("thumbnail worker should start");

    assert_ne!(
        thread_receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap(),
        std::thread::current().id()
    );
    wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(load.receiver.try_recv().unwrap().is_ok());
}

#[test]
fn live_preview_decode_runs_on_the_dedicated_worker() {
    let (directory, path) = temp_file("preview-decode-worker");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let request = match app.state.frame_capture.as_ref().unwrap().preview.state() {
        PreviewState::Debouncing { request, .. } => request.clone(),
        state => panic!("expected debouncing preview, got {state:?}"),
    };
    let (thread_sender, thread_receiver) = mpsc::sync_channel(1);
    let (wake_tx, wake_rx) = mpsc::channel();
    let wake = move || {
        let _ = wake_tx.send(());
    };
    let decode = FramePreviewDecode::spawn_with(request, wake, move || {
        thread_sender.send(std::thread::current().id()).unwrap();
        Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
    })
    .expect("preview decode worker should start");

    assert_ne!(
        thread_receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap(),
        std::thread::current().id()
    );
    wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(decode.receiver.try_recv().unwrap().is_ok());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn authoritative_preview_texture_is_stable_across_unrelated_repaints() {
    let (directory, path) = temp_file("preview-texture-stability");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let request = session
        .preview
        .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
        .unwrap();
    assert!(session.preview.settle(
        request,
        Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
    ));

    app.sync_frame_preview_texture();
    let first_texture = app
        .state
        .frame_capture
        .as_ref()
        .unwrap()
        .preview_texture
        .as_ref()
        .unwrap()
        .handle
        .id();
    app.egui_context.request_repaint();
    app.sync_frame_preview_texture();
    let second_texture = app
        .state
        .frame_capture
        .as_ref()
        .unwrap()
        .preview_texture
        .as_ref()
        .unwrap()
        .handle
        .id();

    assert_eq!(first_texture, second_texture);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_timestamp_retires_work_but_keeps_the_last_valid_preview_visible() {
    let (directory, path) = temp_file("preview-invalid-input-retains-texture");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let request = session
        .preview
        .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
        .unwrap();
    assert!(session.preview.settle(
        request,
        Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
    ));
    app.sync_frame_preview_texture();

    app.update_frame_capture_position(None, PositionSettle::Debounced, Instant::now());

    let session = app.state.frame_capture.as_ref().unwrap();
    assert!(matches!(session.preview.state(), PreviewState::Idle));
    assert!(session.preview_texture.is_some());
    assert!(session.frame_index_due.is_none());
    assert!(matches!(
        session.frame_navigator.state(),
        FrameIndexState::Idle
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn navigation_enablement_uses_exact_sub_millisecond_frame_identity() {
    let (directory, path) = temp_file("sub-millisecond-navigation-boundaries");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let half_millisecond = ResolvedFramePosition::new(
        1,
        FrameTimeBase::new(1, 2_000).expect("valid test time base"),
    )
    .expect("displayable test frame");
    {
        let session = app.state.frame_capture.as_mut().unwrap();
        session.frame_index_due = None;
        let FrameCaptureToolState { media, editor, .. } = session;
        editor
            .selection
            .set_position(media, FramePosition::from(half_millisecond));
        let request = editor
            .frame_navigator
            .schedule(
                editor.source_identity.as_ref().unwrap(),
                media,
                FramePosition::from(half_millisecond),
            )
            .unwrap()
            .unwrap();
        assert!(editor.frame_navigator.settle(
            request,
            Ok(br#"{"frames":[{"best_effort_timestamp":0},{"best_effort_timestamp":1}],"streams":[{"time_base":"1/2000"}]}"#.to_vec())
        ));
    }

    let session = app.state.frame_capture.as_ref().unwrap();
    let availability = frame_navigation_availability(&session.media, &session.editor);
    assert!(availability.previous);
    assert!(!availability.next);
    assert!(!availability.loading);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn position_intent_uses_debounce_while_release_is_immediately_due() {
    let (directory, path) = temp_file("position-settle-policy");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let position = FrameTimestamp::from_milliseconds(250).into();
    seed_frame_neighborhood(
        &mut app,
        FrameTimestamp::ZERO.into(),
        br#"{"frames":[{"best_effort_timestamp":0},{"best_effort_timestamp":100}],"streams":[{"time_base":"1/1000"}]}"#,
    );

    let now = Instant::now();
    app.update_frame_capture_position(Some(position), PositionSettle::Debounced, now);
    let session = app.state.frame_capture.as_ref().unwrap();
    let debounced_ready_at = match session.preview.state() {
        PreviewState::Debouncing { ready_at, .. } => *ready_at,
        state => panic!("expected debouncing preview, got {state:?}"),
    };
    assert_eq!(debounced_ready_at, now + PREVIEW_DEBOUNCE);
    assert_eq!(
        session.frame_index_due,
        Some((position, debounced_ready_at))
    );
    assert!(matches!(
        session.frame_navigator.state(),
        FrameIndexState::Idle
    ));

    app.update_frame_capture_position(Some(position), PositionSettle::Immediate, now);
    let session = app.state.frame_capture.as_ref().unwrap();
    let immediate_ready_at = match session.preview.state() {
        PreviewState::Debouncing { ready_at, .. } => *ready_at,
        state => panic!("expected immediately due preview, got {state:?}"),
    };
    assert_eq!(immediate_ready_at, now);
    assert_eq!(session.frame_index_due, Some((position, now)));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cached_vfr_steps_reuse_navigation_coverage_and_schedule_only_preview() {
    let (directory, path) = temp_file("vfr-frame-step");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let time_base = FrameTimeBase::new(1, 1_000).expect("valid test time base");
    seed_frame_neighborhood(
        &mut app,
        FrameTimestamp::from_milliseconds(100).into(),
        br#"{"frames":[{"best_effort_timestamp":40},{"best_effort_timestamp":100},{"best_effort_timestamp":170},{"best_effort_timestamp":240},{"best_effort_timestamp":310},{"best_effort_timestamp":380}],"streams":[{"time_base":"1/1000"}]}"#,
    );

    app.request_frame_step(FrameDirection::Next);
    assert_cached_step(&mut app, 170, time_base);

    app.request_frame_step(FrameDirection::Next);
    assert_cached_step(&mut app, 240, time_base);

    app.state
        .frame_capture
        .as_mut()
        .unwrap()
        .selection
        .set_step(FrameStep::Two);
    app.request_frame_step(FrameDirection::Next);
    assert_cached_step(&mut app, 380, time_base);

    app.request_frame_step(FrameDirection::Previous);
    assert_cached_step(&mut app, 240, time_base);
    fs::remove_dir_all(directory).unwrap();
}

fn assert_cached_step(app: &mut ApolloApp, timestamp: i64, time_base: FrameTimeBase) {
    let expected = ResolvedFramePosition::new(timestamp, time_base)
        .expect("stepped test frame remains displayable");
    let session = app.state.frame_capture.as_ref().unwrap();
    assert_eq!(
        session.selection.request(&session.media).unwrap().position,
        expected.into()
    );
    assert_eq!(
        session.selection.timestamp_text(),
        expected.display_timestamp().to_string()
    );
    assert!(session.frame_index_due.is_none());
    assert!(matches!(
        session.frame_navigator.state(),
        FrameIndexState::Ready { .. }
    ));
    assert!(matches!(
        session.preview.state(),
        PreviewState::Debouncing { request, ready_at }
            if request.position == expected.into() && *ready_at <= Instant::now()
    ));
    assert!(!app.frame_index_query.cancel());
}

#[test]
fn leaving_cached_frame_coverage_starts_one_new_neighborhood_query() {
    let (directory, path) = temp_file("frame-step-needs-neighborhood");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    seed_frame_neighborhood(
        &mut app,
        FrameTimestamp::from_milliseconds(100).into(),
        br#"{"frames":[{"best_effort_timestamp":40},{"best_effort_timestamp":100},{"best_effort_timestamp":170},{"best_effort_timestamp":240}],"streams":[{"time_base":"1/1000"}]}"#,
    );
    {
        let session = app.state.frame_capture.as_mut().unwrap();
        session.selection.set_step(FrameStep::Two);
    }
    app.request_frame_step(FrameDirection::Next);
    assert_cached_step(
        &mut app,
        240,
        FrameTimeBase::new(1, 1_000).expect("valid test time base"),
    );
    app.state
        .frame_capture
        .as_mut()
        .unwrap()
        .selection
        .set_step(FrameStep::One);
    app.core.discovery = MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
        "missing-ffmpeg",
        std::env::current_exe().expect("current test executable is available"),
    )));

    app.request_frame_step(FrameDirection::Next);

    let edge = ResolvedFramePosition::new(
        240,
        FrameTimeBase::new(1, 1_000).expect("valid test time base"),
    )
    .expect("edge frame remains displayable");
    let request = {
        let session = app.state.frame_capture.as_ref().unwrap();
        assert!(session.frame_index_due.is_none());
        assert!(session.pending_step.is_some());
        match session.frame_navigator.state() {
            FrameIndexState::Loading { request } => {
                assert_eq!(request.center, edge.into());
                request.clone()
            }
            state => panic!("expected one loading edge query, got {state:?}"),
        }
    };
    app.frame_index_query.cancel();
    app.settle_frame_index_request(request, Err(FrameIndexError::Cancelled));

    let session = app.state.frame_capture.as_ref().unwrap();
    assert!(session.pending_step.is_none());
    assert!(matches!(
        session.frame_navigator.state(),
        FrameIndexState::Failed {
            request,
            error: FrameIndexError::Cancelled,
        } if request.center == edge.into()
    ));
    assert!(!app.frame_index_query.cancel());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cached_timestamp_becomes_ready_without_launching_ffmpeg() {
    let (directory, path) = temp_file("preview-cache-hit");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let FrameCaptureToolState { media, editor, .. } = session;
    let first = editor
        .preview
        .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
        .unwrap();
    assert!(editor.preview.settle(
        first,
        Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
    ));
    let cached_generation = editor
        .preview
        .schedule(
            editor.source_identity.as_ref().unwrap(),
            media,
            FrameTimestamp::ZERO,
            Instant::now() - PREVIEW_DEBOUNCE,
        )
        .unwrap();

    app.poll_frame_capture_preview();

    assert!(matches!(
        app.state.frame_capture.as_ref().unwrap().preview.state(),
        PreviewState::Ready { request, image }
            if request.generation == cached_generation && image.size() == [2, 1]
    ));
    assert!(app.frame_preview_extraction.poll().is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_decode_completion_is_rejected_after_a_newer_edit() {
    let (directory, path) = temp_file("preview-stale-decode");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let FrameCaptureToolState { media, editor, .. } = session;
    let stale_request = editor
        .preview
        .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
        .unwrap();
    let current_generation = editor
        .preview
        .schedule(
            editor.source_identity.as_ref().unwrap(),
            media,
            FrameTimestamp::from_milliseconds(500),
            Instant::now(),
        )
        .unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    app.frame_preview_decode = Some(FramePreviewDecode {
        request: stale_request,
        receiver,
    });
    sender
        .send(Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap()))
        .unwrap();

    app.poll_frame_preview_decode();

    assert!(app.frame_preview_decode.is_none());
    assert!(matches!(
        app.state.frame_capture.as_ref().unwrap().preview.state(),
        PreviewState::Debouncing { request, .. }
            if request.generation == current_generation
                && request.position.display_timestamp()
                    == FrameTimestamp::from_milliseconds(500)
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn thumbnail_completion_routes_to_its_originating_frame_capture_session() {
    let (directory, path) = temp_file("thumbnail-routing");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let session_id = session.session_id;
    let request = session
        .lifecycle_ui
        .begin_thumbnail(Path::new("capture.png"))
        .unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    app.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
        session_id,
        request_id: request.request_id(),
        receiver,
    });

    sender.send(Ok(thumbnail())).unwrap();
    app.poll_frame_capture_thumbnail();

    assert!(app.frame_capture_thumbnail_load.is_none());
    assert!(matches!(
        app.state
            .frame_capture
            .as_ref()
            .unwrap()
            .lifecycle_ui
            .thumbnail(),
        FrameCaptureThumbnailState::Ready { request_id, .. }
            if *request_id == request.request_id()
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn frame_preview_ownership_follows_source_and_operation_sessions() {
    use crate::frame_capture::preview::PreviewState;

    let (first_directory, first_path) = temp_file("preview-first-source");
    let (second_directory, second_path) = temp_file("preview-second-source");
    let mut app = app();
    app.load_media_path_with_probe(&first_path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);

    let first_generation = match app
        .state
        .frame_capture
        .as_ref()
        .map(|session| session.preview.state())
    {
        Some(PreviewState::Debouncing { request, .. }) => {
            assert_eq!(request.input, first_path);
            assert_eq!(
                &request.source_identity,
                app.state
                    .frame_capture
                    .as_ref()
                    .unwrap()
                    .source_identity
                    .as_ref()
                    .unwrap()
            );
            request.generation
        }
        state => panic!("expected a debounced preview for the first source, got {state:?}"),
    };

    app.select_tool(Operation::Remux);
    assert!(matches!(
        app.state
            .frame_capture
            .as_ref()
            .map(|session| session.preview.state()),
        Some(PreviewState::Idle)
    ));

    app.select_tool(Operation::FrameCapture);
    assert!(matches!(
        app.state
            .frame_capture
            .as_ref()
            .map(|session| session.preview.state()),
        Some(PreviewState::Debouncing { request, .. })
            if request.generation.get() > first_generation.get()
                && request.input == first_path
    ));

    let old_session_id = app.state.frame_capture.as_ref().unwrap().session_id;
    app.load_media_path_with_probe(&second_path, |path| Ok(media(path)));
    assert!(matches!(
        app.state.frame_capture.as_ref(),
        Some(session)
            if session.session_id != old_session_id
                && matches!(
                    session.preview.state(),
                    PreviewState::Debouncing { request, .. }
                        if request.input == second_path
                )
    ));

    fs::remove_dir_all(first_directory).unwrap();
    fs::remove_dir_all(second_directory).unwrap();
}

#[test]
fn missing_video_preview_request_schedules_no_delayed_repaint() {
    let (directory, path) = temp_file("preview-missing-video-repaint");
    let mut app = app();
    let (delayed_sender, delayed_receiver) = mpsc::channel();
    app.egui_context.set_request_repaint_callback(move |info| {
        if info.delay > Duration::ZERO {
            let _ = delayed_sender.send(info.delay);
        }
    });
    let mut source = media(&path);
    source.video_streams.clear();
    app.load_media_path_with_probe(&path, |_| Ok(source));

    app.select_tool(Operation::FrameCapture);

    assert!(matches!(
        app.state
            .frame_capture
            .as_ref()
            .map(|session| session.preview.state()),
        Some(crate::frame_capture::preview::PreviewState::Idle)
    ));
    assert!(matches!(
        delayed_receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    fs::remove_dir_all(directory).unwrap();
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
fn frame_capture_defaults_apply_before_per_source_timestamp_validation() {
    let (first_directory, first_path) = temp_file("batch-short-video-first");
    let (second_directory, second_path) = temp_file("batch-long-video-second");
    let mut first_media = media(&first_path);
    first_media.duration = Some(Duration::from_secs(5));
    let mut second_media = media(&second_path);
    second_media.duration = Some(Duration::from_secs(20));
    let mut app = app();
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::FrameCapture,
        defaults: Some(BatchDefaults::FrameCapture {
            timestamp: FrameTimestamp::ZERO,
            format: FrameCaptureFormat::Png,
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
    });
    app.core.show_batch_setup();

    assert!(app.begin_batch_edit(BatchEditTarget::Defaults));
    let edit = app.batch_edit.as_mut().unwrap();
    let representative = edit.media.clone();
    edit.frame_capture
        .set_timestamp_text(&representative, "00:00:10.000".to_owned());
    edit.frame_capture.set_format(FrameCaptureFormat::Jpeg);
    assert!(app.save_batch_edit());

    let batch = app.batch_setup.as_ref().unwrap();
    assert_eq!(batch.items[0].readiness, BatchReadiness::NeedsAttention);
    assert_eq!(
        batch.items[0].reason.as_deref(),
        Some("Timestamp is outside this source's duration.")
    );
    assert_eq!(batch.items[1].readiness, BatchReadiness::Ready);
    assert!(matches!(
        batch.items[1].operation.as_ref(),
        Some(QueuedOperation::FrameCapture(request))
            if request.position.to_string() == "00:00:10.000"
                && request.format == FrameCaptureFormat::Jpeg
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
    let selection = crate::ui::remux::RemuxSelectionState::new(&source);
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
fn batch_frame_capture_defaults_stay_simple_while_item_override_owns_precision_support() {
    let (directory, path) = temp_file("batch-frame-capture-preview");
    let source = media(&path);
    let request = FrameCaptureSelectionState::new(&source)
        .request(&source)
        .unwrap();
    let mut app = app();
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::FrameCapture,
        defaults: Some(BatchDefaults::FrameCapture {
            timestamp: FrameTimestamp::ZERO,
            format: FrameCaptureFormat::Png,
        }),
        items: vec![BatchItem {
            path: path.clone(),
            included: true,
            source: ready_batch_source(source),
            operation: Some(QueuedOperation::FrameCapture(request)),
            readiness: BatchReadiness::Ready,
            reason: None,
            overridden: false,
        }],
        error: None,
        append_error: None,
        append_error_details_open: false,
    });
    app.core.show_batch_setup();
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    assert!(app.begin_batch_edit(BatchEditTarget::Defaults));
    let defaults = app.batch_edit.as_ref().unwrap();
    assert!(defaults.frame_capture.source_identity.is_none());
    assert!(matches!(
        defaults.frame_capture.preview.state(),
        PreviewState::Idle
    ));
    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
    assert!(output_contains_text(&output, "Timestamp"));
    assert!(!output_contains_text(&output, "1 frame"));
    output.textures_delta.clear();
    assert!(app.finish_batch_edit());

    assert!(app.begin_batch_edit(BatchEditTarget::Item(0)));
    let item_edit = app.batch_edit.as_ref().unwrap();
    assert!(item_edit.frame_capture.source_identity.is_some());
    assert!(matches!(
        item_edit.frame_capture.preview.state(),
        PreviewState::Debouncing { .. }
    ));
    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
    assert!(output_contains_text(&output, "1 frame"));
    assert!(!output_contains_text(&output, "Timestamp"));
    output.textures_delta.clear();
    seed_frame_neighborhood(
        &mut app,
        FrameTimestamp::from_milliseconds(100).into(),
        br#"{"frames":[{"best_effort_timestamp":100},{"best_effort_timestamp":170}],"streams":[{"time_base":"1/1000"}]}"#,
    );
    app.request_frame_step(FrameDirection::Next);
    let selected = app
        .batch_edit
        .as_ref()
        .unwrap()
        .frame_capture
        .valid_position(&app.batch_edit.as_ref().unwrap().media)
        .unwrap();
    assert!(app.save_batch_edit());
    let Some(QueuedOperation::FrameCapture(saved)) = app.batch_setup.as_ref().unwrap().items[0]
        .operation
        .as_ref()
    else {
        panic!("batch item must save a Frame Capture request");
    };
    assert_eq!(saved.position, selected);
    assert!(!app.core.source.is_inspecting());
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
fn opening_frame_capture_source_renders_ready_surface() {
    let (directory, path) = temp_file("frame-capture-ready-render");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| {
        show_tool(&mut app, ui, Operation::FrameCapture);
    });
    output.textures_delta.clear();
    for text in [
        "recording.mkv",
        "1920×1080",
        "00:00:00.000",
        "1 frame",
        "PNG",
        "lossless",
        "Output",
        "Browse",
        "Capture frame",
    ] {
        assert!(output_contains_text(&output, text), "missing {text:?}");
    }
    assert!(!output_contains_text(&output, "Format"));
    assert!(!output_contains_text(&output, "Timestamp"));
    assert!(!output_contains_text(&output, "Duration"));
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
fn audio_mix_output_dialog_request_follows_current_container() {
    let (directory, path) = temp_file("audio-browse");
    let mut app = app();
    app.select_tool(Operation::AudioMix);
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));

    let session = app.state.audio_mix.as_ref().unwrap();
    let request = audio_mix_output_dialog_request(session);
    let FileDialogRequest::SaveOutput { target, options } = request else {
        panic!("audio browse must create a save request");
    };
    assert_eq!(
        target,
        DialogTarget::AudioMixOutput {
            session_id: session.session_id,
        }
    );
    assert_eq!(options.initial_directory, Some(directory.clone()));
    assert_eq!(options.file_name, "recording-mixed.mp4");
    assert_eq!(options.format, OutputFormat::Mp4);

    let session = app.state.audio_mix.as_mut().unwrap();
    let media = session.media.clone();
    session
        .selection
        .set_container(&media, OutputContainer::Mkv);
    let request = audio_mix_output_dialog_request(session);
    let FileDialogRequest::SaveOutput { options, .. } = request else {
        unreachable!();
    };
    assert_eq!(options.file_name, "recording-mixed.mkv");
    assert_eq!(options.format, OutputFormat::Mkv);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn remux_output_dialog_request_follows_current_container() {
    let (directory, path) = temp_file("remux-browse");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);

    let session = app.state.remux.as_ref().unwrap();
    let FileDialogRequest::SaveOutput { target, options } = remux_output_dialog_request(session)
    else {
        panic!("remux browse must create a save request");
    };
    assert_eq!(
        target,
        DialogTarget::RemuxOutput {
            session_id: session.session_id,
        }
    );
    assert_eq!(options.initial_directory, Some(directory.clone()));
    assert_eq!(options.file_name, "recording-remuxed.mkv");
    assert_eq!(options.format, OutputFormat::Mkv);

    let session = app.state.remux.as_mut().unwrap();
    session.selection.set_container(MediaContainer::Mp4);
    let FileDialogRequest::SaveOutput { options, .. } = remux_output_dialog_request(session) else {
        unreachable!();
    };
    assert_eq!(options.file_name, "recording-remuxed.mp4");
    assert_eq!(options.format, OutputFormat::Mp4);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn audio_extract_output_dialog_request_follows_current_format() {
    let (directory, path) = temp_file("audio-extract-browse");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::AudioExtract);

    let session = app.state.audio_extract.as_ref().unwrap();
    let FileDialogRequest::SaveOutput { target, options } =
        audio_extract_output_dialog_request(session)
    else {
        panic!("audio extract Browse must create a save request");
    };
    assert_eq!(
        target,
        DialogTarget::AudioExtractOutput {
            session_id: session.session_id,
        }
    );
    assert_eq!(options.initial_directory, Some(directory.clone()));
    assert_eq!(options.file_name, "recording-audio.mka");
    assert_eq!(options.format, OutputFormat::Mka);

    let session = app.state.audio_extract.as_mut().unwrap();
    for (format, name, dialog_format) in [
        (
            AudioExtractFormat::Aac,
            "recording-audio.m4a",
            OutputFormat::M4a,
        ),
        (
            AudioExtractFormat::Flac,
            "recording-audio.flac",
            OutputFormat::Flac,
        ),
    ] {
        session.selection.set_format(format);
        let FileDialogRequest::SaveOutput { options, .. } =
            audio_extract_output_dialog_request(session)
        else {
            unreachable!();
        };
        assert_eq!(options.file_name, name);
        assert_eq!(options.format, dialog_format);
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn frame_capture_output_dialog_request_follows_current_format() {
    let (directory, path) = temp_file("frame-capture-browse");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);

    let session = app.state.frame_capture.as_ref().unwrap();
    let FileDialogRequest::SaveOutput { target, options } =
        frame_capture_output_dialog_request(session)
    else {
        panic!("frame capture Browse must create a save request");
    };
    assert_eq!(
        target,
        DialogTarget::FrameCaptureOutput {
            session_id: session.session_id,
        }
    );
    assert_eq!(options.initial_directory, Some(directory.clone()));
    assert_eq!(options.file_name, "recording-frame-00-00-00-000.png");
    assert_eq!(options.format, OutputFormat::Png);

    let session = app.state.frame_capture.as_mut().unwrap();
    session.selection.set_format(FrameCaptureFormat::Jpeg);
    let FileDialogRequest::SaveOutput { options, .. } =
        frame_capture_output_dialog_request(session)
    else {
        unreachable!();
    };
    assert_eq!(options.file_name, "recording-frame-00-00-00-000.jpg");
    assert_eq!(options.format, OutputFormat::Jpeg);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dialog_requests_prefer_independent_remembered_directories() {
    let (source_directory, source_path) = temp_file("remember-source");
    let output_directory = source_directory.join("remembered-output");
    fs::create_dir_all(&output_directory).unwrap();
    let mut app = app();
    app.settings
        .remember_source(&source_directory.join("previous-source.mp4"));
    app.settings
        .remember_output(&output_directory.join("previous-output.mkv"));

    let open = app.prepare_file_dialog_request(FileDialogRequest::OpenSource {
        initial_directory: None,
    });
    assert_eq!(
        open,
        FileDialogRequest::OpenSource {
            initial_directory: Some(source_directory.clone()),
        }
    );

    app.load_media_path_with_probe(&source_path, |path| Ok(media(path)));
    let save = app.prepare_file_dialog_request(audio_mix_output_dialog_request(
        app.state.audio_mix.as_ref().unwrap(),
    ));
    let FileDialogRequest::SaveOutput { options, .. } = save else {
        unreachable!();
    };
    assert_eq!(options.initial_directory, Some(output_directory));
    fs::remove_dir_all(source_directory).unwrap();
}

#[test]
fn dialog_cancel_does_not_change_directory_memory() {
    let (directory, path) = temp_file("remember-cancel");
    let mut app = app();
    app.settings.remember_source(&path);
    app.settings
        .remember_output(&directory.join("existing.mkv"));
    let before = app.settings.clone();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::MediaInput,
        outcome: DialogOutcome::Cancelled,
    });
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioMixOutput { session_id: 1 },
        outcome: DialogOutcome::Cancelled,
    });

    assert_eq!(app.settings, before);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn successful_dialog_selections_update_only_their_directory_history() {
    let (directory, source_path) = temp_file("remember-success");
    let output_directory = directory.join("output");
    fs::create_dir_all(&output_directory).unwrap();
    let mut app = app();
    app.core.discovery =
        MediaToolsDiscovery::with_state(MediaToolsState::Unavailable(MediaToolsError::MissingBoth));

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::MediaInput,
        outcome: DialogOutcome::Selected(source_path),
    });
    assert_eq!(app.settings.source_directory(), Some(directory.clone()));
    assert_eq!(app.settings.output_directory(Path::new("missing")), None);

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioMixOutput { session_id: 999 },
        outcome: DialogOutcome::Selected(output_directory.join("chosen.mkv")),
    });
    assert_eq!(app.settings.source_directory(), Some(directory.clone()));
    assert_eq!(
        app.settings.output_directory(Path::new("missing")),
        Some(output_directory)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn selected_output_paths_route_to_the_owning_tool() {
    let (directory, path) = temp_file("upscale-browse");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);
    let upscale_session_id = app.state.upscale.as_ref().unwrap().session_id;
    let destination = directory.join("chosen upscale.mp4");
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::UpscaleOutput {
            session_id: upscale_session_id,
        },
        outcome: DialogOutcome::Selected(destination.clone()),
    });

    assert_eq!(
        app.state.upscale.as_ref().unwrap().selection.output_path(),
        destination
    );

    app.select_tool(Operation::AudioMix);
    let audio_session_id = app.state.audio_mix.as_ref().unwrap().session_id;
    let audio_destination = directory.join("chosen mix.mp4");
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioMixOutput {
            session_id: audio_session_id,
        },
        outcome: DialogOutcome::Selected(audio_destination.clone()),
    });
    assert_eq!(
        app.state
            .audio_mix
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        audio_destination
    );

    app.select_tool(Operation::Remux);
    let remux_session_id = app.state.remux.as_ref().unwrap().session_id;
    let remux_destination = directory.join("chosen remux.mkv");
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::RemuxOutput {
            session_id: remux_session_id,
        },
        outcome: DialogOutcome::Selected(remux_destination.clone()),
    });
    assert_eq!(
        app.state.remux.as_ref().unwrap().selection.output_path(),
        remux_destination
    );

    app.select_tool(Operation::AudioExtract);
    let extract_session_id = app.state.audio_extract.as_ref().unwrap().session_id;
    let extract_destination = directory.join("chosen audio.mka");
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioExtractOutput {
            session_id: extract_session_id,
        },
        outcome: DialogOutcome::Selected(extract_destination.clone()),
    });
    assert_eq!(
        app.state
            .audio_extract
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        extract_destination
    );

    app.select_tool(Operation::FrameCapture);
    let capture_session_id = app.state.frame_capture.as_ref().unwrap().session_id;
    let capture_destination = directory.join("chosen frame.png");
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::FrameCaptureOutput {
            session_id: capture_session_id,
        },
        outcome: DialogOutcome::Selected(capture_destination.clone()),
    });
    assert_eq!(
        app.state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        capture_destination
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_audio_extract_completion_cannot_mutate_replacement_session() {
    let (directory, first) = temp_file("stale-audio-extract");
    let second = directory.join("replacement.mkv");
    fs::write(&second, b"replacement").unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&first, |path| Ok(media(path)));
    app.select_tool(Operation::AudioExtract);
    let stale_session_id = app.state.audio_extract.as_ref().unwrap().session_id;

    app.load_media_path_with_probe(&second, |path| Ok(media(path)));
    let replacement_before = app
        .state
        .audio_extract
        .as_ref()
        .unwrap()
        .selection
        .output_path();
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioExtractOutput {
            session_id: stale_session_id,
        },
        outcome: DialogOutcome::Selected(directory.join("stale.mka")),
    });
    assert_eq!(
        app.state
            .audio_extract
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        replacement_before
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_frame_capture_completion_cannot_mutate_replacement_session() {
    let (directory, first) = temp_file("stale-frame-capture");
    let second = directory.join("replacement.mkv");
    fs::write(&second, b"replacement").unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&first, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let stale_session_id = app.state.frame_capture.as_ref().unwrap().session_id;

    app.load_media_path_with_probe(&second, |path| Ok(media(path)));
    let replacement_before = app
        .state
        .frame_capture
        .as_ref()
        .unwrap()
        .selection
        .output_path();
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::FrameCaptureOutput {
            session_id: stale_session_id,
        },
        outcome: DialogOutcome::Selected(directory.join("stale.png")),
    });
    assert_eq!(
        app.state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        replacement_before
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_remux_output_completion_cannot_mutate_replacement_session() {
    let (directory, first) = temp_file("stale-remux");
    let second = directory.join("replacement.mkv");
    fs::write(&second, b"replacement").unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&first, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let stale_session_id = app.state.remux.as_ref().unwrap().session_id;

    app.load_media_path_with_probe(&second, |path| Ok(media(path)));
    let replacement_before = app.state.remux.as_ref().unwrap().selection.output_path();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::RemuxOutput {
            session_id: stale_session_id,
        },
        outcome: DialogOutcome::Selected(directory.join("stale-result.mkv")),
    });

    let replacement = app.state.remux.as_ref().unwrap();
    assert_ne!(replacement.session_id, stale_session_id);
    assert_eq!(replacement.selection.output_path(), replacement_before);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cancelled_output_browse_preserves_ready_state() {
    let (directory, path) = temp_file("browse-cancel");
    let mut app = app();
    app.select_tool(Operation::AudioMix);
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let before = app.state.clone();
    let session_id = app.state.audio_mix.as_ref().unwrap().session_id;

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioMixOutput { session_id },
        outcome: DialogOutcome::Cancelled,
    });

    assert_eq!(app.state, before);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pending_dialog_blocks_conflicting_app_actions_and_a_second_request() {
    let (directory, path) = temp_file("pending-dialog-boundary");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let (release_sender, release_receiver) = mpsc::channel();
    let mut release_receiver = Some(release_receiver);
    let context = egui::Context::default();
    let (wake_tx, wake_rx) = mpsc::channel();
    context.set_request_repaint_callback(move |_| {
        let _ = wake_tx.send(());
    });
    app.file_dialog = FileDialogOwner::new_for_test(completion_wake(&context), move || {
        Ok(move |_: &FileDialogRequest| {
            if let Some(receiver) = release_receiver.take() {
                receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            Ok(None)
        })
    });
    app.file_dialog
        .request(FileDialogRequest::OpenSource {
            initial_directory: None,
        })
        .unwrap();

    assert!(!app.file_intake_enabled());
    assert!(app.workspace_interaction_enabled());
    assert_workspace(&app.core, Operation::AudioMix);
    assert_eq!(
        app.file_dialog.request(FileDialogRequest::OpenSource {
            initial_directory: None,
        }),
        Err(FileDialogStartError::AlreadyPending)
    );

    release_sender.send(()).unwrap();
    wake_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("dialog completion must wake the UI");
    assert_eq!(
        app.file_dialog.poll().unwrap().outcome,
        DialogOutcome::Cancelled
    );
    assert!(!app.file_dialog.is_pending());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_output_completions_cannot_mutate_replacement_sessions() {
    let (directory, first) = temp_file("stale-dialog-session");
    let second = directory.join("replacement.mkv");
    let third = directory.join("third.mkv");
    fs::write(&second, b"replacement").unwrap();
    fs::write(&third, b"third").unwrap();
    let mut app = app();

    app.select_tool(Operation::AudioMix);
    app.load_media_path_with_probe(&first, |path| Ok(media(path)));
    let old_audio_id = app.state.audio_mix.as_ref().unwrap().session_id;
    app.load_media_path_with_probe(&second, |path| Ok(media(path)));
    let replacement_audio = app.state.audio_mix.as_ref().unwrap();
    let replacement_audio_id = replacement_audio.session_id;
    let replacement_audio_output = replacement_audio.selection.output_path();
    assert_ne!(old_audio_id, replacement_audio_id);

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioMixOutput {
            session_id: old_audio_id,
        },
        outcome: DialogOutcome::Selected(directory.join("stale-mix.mp4")),
    });
    assert_eq!(
        app.state
            .audio_mix
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        replacement_audio_output
    );

    app.select_tool(Operation::VideoUpscale);
    let old_upscale_id = app.state.upscale.as_ref().unwrap().session_id;
    app.load_media_path_with_probe(&third, |path| Ok(media(path)));
    let replacement_upscale = app.state.upscale.as_ref().unwrap();
    let replacement_upscale_id = replacement_upscale.session_id;
    let replacement_upscale_output = replacement_upscale.selection.output_path();
    assert_ne!(old_upscale_id, replacement_upscale_id);

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::UpscaleOutput {
            session_id: old_upscale_id,
        },
        outcome: DialogOutcome::Selected(directory.join("stale-upscale.mp4")),
    });
    assert_eq!(
        app.state.upscale.as_ref().unwrap().selection.output_path(),
        replacement_upscale_output
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn output_completion_cannot_mutate_selection_after_execution_starts() {
    let (directory, path) = temp_file("dialog-after-execution");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let audio_id = app.state.audio_mix.as_ref().unwrap().session_id;
    let audio_output = app
        .state
        .audio_mix
        .as_ref()
        .unwrap()
        .selection
        .output_path();

    settle_output_file_dialog(
        &mut app.state,
        ToolExecutionStates {
            audio_mix: &AudioMixExecutionState::Running,
            upscale: &UpscaleExecutionState::Idle,
            remux: &RemuxExecutionState::Idle,
            audio_extract: &AudioExtractExecutionState::Idle,
            frame_capture: &FrameCaptureExecutionState::Idle,
        },
        DialogTarget::AudioMixOutput {
            session_id: audio_id,
        },
        DialogOutcome::Selected(directory.join("late-mix.mp4")),
    );
    assert_eq!(
        app.state
            .audio_mix
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        audio_output
    );

    app.select_tool(Operation::VideoUpscale);
    let upscale_id = app.state.upscale.as_ref().unwrap().session_id;
    let upscale_output = app.state.upscale.as_ref().unwrap().selection.output_path();
    settle_output_file_dialog(
        &mut app.state,
        ToolExecutionStates {
            audio_mix: &AudioMixExecutionState::Idle,
            upscale: &UpscaleExecutionState::Running,
            remux: &RemuxExecutionState::Idle,
            audio_extract: &AudioExtractExecutionState::Idle,
            frame_capture: &FrameCaptureExecutionState::Idle,
        },
        DialogTarget::UpscaleOutput {
            session_id: upscale_id,
        },
        DialogOutcome::Selected(directory.join("late-upscale.mp4")),
    );
    assert_eq!(
        app.state.upscale.as_ref().unwrap().selection.output_path(),
        upscale_output
    );

    app.select_tool(Operation::AudioExtract);
    let extract_id = app.state.audio_extract.as_ref().unwrap().session_id;
    let extract_output = app
        .state
        .audio_extract
        .as_ref()
        .unwrap()
        .selection
        .output_path();
    settle_output_file_dialog(
        &mut app.state,
        ToolExecutionStates {
            audio_mix: &AudioMixExecutionState::Idle,
            upscale: &UpscaleExecutionState::Idle,
            remux: &RemuxExecutionState::Idle,
            audio_extract: &AudioExtractExecutionState::Running,
            frame_capture: &FrameCaptureExecutionState::Idle,
        },
        DialogTarget::AudioExtractOutput {
            session_id: extract_id,
        },
        DialogOutcome::Selected(directory.join("late-audio.mka")),
    );
    assert_eq!(
        app.state
            .audio_extract
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        extract_output
    );

    app.select_tool(Operation::FrameCapture);
    let capture_id = app.state.frame_capture.as_ref().unwrap().session_id;
    let capture_output = app
        .state
        .frame_capture
        .as_ref()
        .unwrap()
        .selection
        .output_path();
    settle_output_file_dialog(
        &mut app.state,
        ToolExecutionStates {
            audio_mix: &AudioMixExecutionState::Idle,
            upscale: &UpscaleExecutionState::Idle,
            remux: &RemuxExecutionState::Idle,
            audio_extract: &AudioExtractExecutionState::Idle,
            frame_capture: &FrameCaptureExecutionState::Running,
        },
        DialogTarget::FrameCaptureOutput {
            session_id: capture_id,
        },
        DialogOutcome::Selected(directory.join("late-frame.png")),
    );
    assert_eq!(
        app.state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        capture_output
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn output_picker_failure_surfaces_bounded_user_feedback() {
    let (directory, path) = temp_file("browse-failure");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);
    let output_before = app
        .state
        .upscale
        .as_ref()
        .unwrap()
        .selection
        .output_path()
        .to_path_buf();
    let session_id = app.state.upscale.as_ref().unwrap().session_id;

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::UpscaleOutput { session_id },
        outcome: DialogOutcome::Failed("native picker diagnostic marker".to_owned()),
    });

    let error = app
        .state
        .upscale
        .as_ref()
        .unwrap()
        .start_error
        .as_ref()
        .expect("picker failure should become user-facing state");
    assert_eq!(error.kind(), MediaErrorKind::OutputPickerUnavailable);
    assert_eq!(error.diagnostic(), Some("native picker diagnostic marker"));
    assert_eq!(
        app.state.upscale.as_ref().unwrap().selection.output_path(),
        output_before
    );

    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    let mut rendered = context.run_ui(Default::default(), |ui| {
        assert!(app.show_upscale_start_error(ui));
    });
    assert!(output_contains_text(&rendered, "Output picker unavailable"));
    assert!(output_contains_text(
        &rendered,
        "The output picker could not be opened."
    ));
    assert!(!output_contains_text(
        &rendered,
        "native picker diagnostic marker"
    ));
    rendered.textures_delta.clear();

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
    let operation = QueuedOperation::Remux(
        crate::ui::remux::RemuxSelectionState::new(&source).request(&source),
    );
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
fn batch_append_picker_failure_is_visible_and_never_leaks_to_intake() {
    let (directory, path) = temp_file("batch-append-picker-failure");
    let source = media(&path);
    let operation = QueuedOperation::Remux(
        crate::ui::remux::RemuxSelectionState::new(&source).request(&source),
    );
    let mut app = app();
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::Remux,
        defaults: Some(BatchDefaults::Remux {
            container: MediaContainer::Matroska,
        }),
        items: vec![BatchItem {
            path,
            included: true,
            source: ready_batch_source(source),
            operation: Some(operation),
            readiness: BatchReadiness::Ready,
            reason: None,
            overridden: true,
        }],
        error: None,
        append_error: None,
        append_error_details_open: false,
    });
    app.core.show_batch_setup();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::BatchAppend,
        outcome: DialogOutcome::Failed("native picker diagnostic".to_owned()),
    });

    assert!(app.core.source.error().is_none());
    let batch = app.batch_setup.as_ref().unwrap();
    assert!(matches!(
        batch.append_error,
        Some(ref error)
            if error.kind() == MediaErrorKind::FilePickerUnavailable
                && error.diagnostic() == Some("native picker diagnostic")
    ));
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
    assert!(output_contains_text(
        &output,
        "The file picker could not be opened."
    ));
    output.textures_delta.clear();

    assert!(app.finish_batch_setup());
    assert!(app.core.source.error().is_none());
    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
    assert!(!output_contains_text(
        &output,
        "The file picker could not be opened."
    ));
    output.textures_delta.clear();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn batch_append_picker_cancellation_preserves_the_complete_batch_state() {
    let (directory, path) = temp_file("batch-append-picker-cancel");
    let source = media(&path);
    let operation = QueuedOperation::Remux(
        crate::ui::remux::RemuxSelectionState::new(&source).request(&source),
    );
    let mut app = app();
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::Remux,
        defaults: Some(BatchDefaults::Remux {
            container: MediaContainer::Matroska,
        }),
        items: vec![BatchItem {
            path,
            included: false,
            source: ready_batch_source(source),
            operation: Some(operation),
            readiness: BatchReadiness::Ready,
            reason: None,
            overridden: true,
        }],
        error: Some("Existing batch feedback".to_owned()),
        append_error: Some(MediaError::file_picker_unavailable(
            "existing append diagnostic",
        )),
        append_error_details_open: true,
    });
    app.core.show_batch_setup();
    let before = app.batch_setup.clone();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::BatchAppend,
        outcome: DialogOutcome::Cancelled,
    });

    assert_eq!(app.batch_setup, before);
    assert!(app.core.source.error().is_none());
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
fn queue_frame_capture_edit_reuses_precision_support_and_saves_exact_position() {
    let (directory, path) = temp_file("queue-frame-capture-preview");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let job_id = app
        .enqueue_current_operation(Operation::FrameCapture)
        .unwrap()
        .unwrap();
    let queued_identity = app.core.media_queue.jobs()[0]
        .retained_intake()
        .current_with_identity()
        .unwrap()
        .0
        .clone();
    app.enter_queue();

    assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
    let edit = app.queue_edit.as_ref().unwrap();
    assert_eq!(
        edit.frame_capture.source_identity.as_ref(),
        Some(&queued_identity)
    );
    assert!(matches!(
        edit.frame_capture.preview.state(),
        PreviewState::Debouncing { request, .. }
            if request.source_identity == queued_identity
    ));
    assert!(!app.core.source.is_inspecting());

    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
    assert!(output_contains_text(&output, "1 frame"));
    assert!(!output_contains_text(&output, "Timestamp"));
    output.textures_delta.clear();

    seed_frame_neighborhood(
        &mut app,
        FrameTimestamp::from_milliseconds(100).into(),
        br#"{"frames":[{"best_effort_timestamp":100},{"best_effort_timestamp":170},{"best_effort_timestamp":240}],"streams":[{"time_base":"1/1000"}]}"#,
    );
    app.request_frame_step(FrameDirection::Next);
    let selected = app
        .queue_edit
        .as_ref()
        .unwrap()
        .frame_capture
        .valid_position(&app.queue_edit.as_ref().unwrap().media)
        .unwrap();
    assert_eq!(
        selected,
        FramePosition::from(
            ResolvedFramePosition::new(170, FrameTimeBase::new(1, 1_000).unwrap()).unwrap()
        )
    );

    assert!(app.save_queue_edit());
    let QueuedOperation::FrameCapture(saved) = app.core.media_queue.jobs()[0].operation() else {
        panic!("frame capture edit must remain Frame Capture");
    };
    assert_eq!(saved.position, selected);
    assert_eq!(app.core.media_queue.jobs()[0].id(), job_id);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_queue_frame_capture_switch_and_cancel_retire_support_work() {
    let (directory, path) = temp_file("workspace-frame-capture-edit-support");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let job_id = app
        .enqueue_current_operation(Operation::AudioMix)
        .unwrap()
        .unwrap();

    assert!(app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
    assert!(app.select_queue_edit_tool(Operation::FrameCapture));
    let stale_request = app
        .queue_edit
        .as_mut()
        .unwrap()
        .frame_capture
        .preview
        .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
        .unwrap();
    assert!(app.select_queue_edit_tool(Operation::Remux));
    let edit = app.queue_edit.as_ref().unwrap();
    assert!(matches!(
        edit.frame_capture.preview.state(),
        PreviewState::Idle
    ));
    assert!(edit.frame_capture.preview_texture.is_none());
    assert!(!app.settle_frame_preview(
        stale_request,
        Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
    ));

    assert!(app.select_queue_edit_tool(Operation::FrameCapture));
    let current_request = match app
        .queue_edit
        .as_ref()
        .unwrap()
        .frame_capture
        .preview
        .state()
    {
        PreviewState::Debouncing { request, .. } => request.clone(),
        state => panic!("expected fresh editor preview, got {state:?}"),
    };
    assert!(app.finish_queue_edit());
    assert!(!app.settle_frame_preview(
        current_request,
        Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
    ));
    assert!(!app.core.source.is_inspecting());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_edit_hydrates_each_operation_from_its_immutable_request() {
    let (directory, path) = temp_file("queue-edit-hydration");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let mut job_ids = Vec::new();
    for operation in [
        Operation::AudioMix,
        Operation::VideoUpscale,
        Operation::Remux,
        Operation::AudioExtract,
        Operation::FrameCapture,
    ] {
        app.select_tool(operation);
        job_ids.push(app.enqueue_current_operation(operation).unwrap().unwrap());
    }
    app.enter_queue();

    for job_id in job_ids {
        let original = app
            .core
            .media_queue
            .jobs()
            .iter()
            .find(|job| job.id() == job_id)
            .unwrap()
            .operation()
            .clone();
        assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
        let edit = app.queue_edit.as_mut().unwrap();
        let hydrated = match &original {
            QueuedOperation::AudioMix(_) => {
                QueuedOperation::AudioMix(edit.audio_mix.request(&edit.media))
            }
            QueuedOperation::Upscale(request) => QueuedOperation::Upscale(
                edit.upscale
                    .request_with_nvenc_capability(&edit.media, request.nvenc_capability.clone()),
            ),
            QueuedOperation::Remux(_) => QueuedOperation::Remux(edit.remux.request(&edit.media)),
            QueuedOperation::AudioExtract(_) => {
                QueuedOperation::AudioExtract(edit.audio_extract.request(&edit.media).unwrap())
            }
            QueuedOperation::FrameCapture(_) => {
                QueuedOperation::FrameCapture(edit.frame_capture.request(&edit.media).unwrap())
            }
        };
        assert_eq!(hydrated, original);
        assert!(app.finish_queue_edit());
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
fn frame_capture_intent_reaches_executor() {
    let (directory, path) = temp_file("frame-capture-execute");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);

    let error = app
        .handle_frame_capture_action(FrameCaptureSelectionAction::CaptureRequested)
        .expect_err("missing test FFmpeg should prove the execution path was attempted");
    assert!(matches!(
        error,
        crate::frame_capture::execution::FrameCaptureStartError::Process(
            crate::media::process::MediaProcessStartError::ProcessLaunchFailed(
                std::io::ErrorKind::NotFound
            )
        )
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn copy_frame_is_output_independent_and_never_creates_a_queue_job() {
    let (directory, path) = temp_file("copy-frame-execute");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let resolved =
        ResolvedFramePosition::new(1001, FrameTimeBase::new(1, 24_000).unwrap()).unwrap();
    let FrameCaptureToolState { media, editor, .. } = session;
    editor.selection.set_position(media, resolved.into());
    editor.selection.set_output_filename_stem(OsString::new());
    assert!(!editor.selection.can_capture(media));
    assert!(copy_frame_action_eligible(media, resolved.into()));

    app.handle_frame_capture_action(FrameCaptureSelectionAction::CopyRequested)
        .unwrap();

    assert!(app.core.media_queue.jobs().is_empty());
    assert_eq!(
        app.core
            .operation_action_error
            .as_ref()
            .map(|(_, error)| error.kind()),
        Some(MediaErrorKind::FfmpegUnavailable)
    );
    assert_eq!(
        app.state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .valid_position(&app.state.frame_capture.as_ref().unwrap().media),
        Some(resolved.into())
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn oversized_copy_reaches_actionable_preflight_without_starting_work_or_mutating_output() {
    let (directory, path) = temp_file("copy-frame-oversized");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let position = session.selection.valid_position(&session.media).unwrap();
    session.media.video_streams[0].width = Some(16_384);
    session.media.video_streams[0].height = Some(16_384);
    session
        .selection
        .set_output_filename_stem(OsString::from("unchanged-output"));
    assert!(copy_frame_action_eligible(&session.media, position));

    app.handle_frame_capture_action(FrameCaptureSelectionAction::CopyRequested)
        .unwrap();

    assert_eq!(
        app.core
            .operation_action_error
            .as_ref()
            .map(|(_, error)| error.kind()),
        Some(MediaErrorKind::FrameTooLargeToCopy)
    );
    assert!(!app.copy_frame_extraction.is_active());
    assert!(app.copy_frame_worker.is_none());
    assert!(app.core.media_queue.jobs().is_empty());
    assert!(app.frame_capture_thumbnail_load.is_none());
    assert_eq!(
        app.state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .output_filename_stem(),
        OsStr::new("unchanged-output")
    );
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

#[test]
fn frame_capture_state_survives_operation_switching() {
    let (directory, path) = temp_file("frame-capture-back");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app
        .state
        .frame_capture
        .as_mut()
        .expect("frame capture session should exist");
    let inspected_media = media(&path);
    session
        .selection
        .set_timestamp_text(&inspected_media, "00:00:03.250".into());
    session.selection.set_format(FrameCaptureFormat::Jpeg);
    let custom_output = directory.join("chosen-frame.jpg");
    session
        .selection
        .set_custom_output_path(custom_output.clone());

    app.select_tool(Operation::AudioMix);
    app.select_tool(Operation::FrameCapture);
    let session = app
        .state
        .frame_capture
        .as_ref()
        .expect("same-source Frame Capture session should be restored");
    assert_workspace(&app.core, Operation::FrameCapture);
    assert_eq!(session.selection.timestamp_text(), "00:00:03.250");
    assert_eq!(session.selection.format(), FrameCaptureFormat::Jpeg);
    assert_eq!(session.selection.output_path(), custom_output);
    assert!(session.selection.output_is_customized());
    fs::remove_dir_all(directory).unwrap();
}

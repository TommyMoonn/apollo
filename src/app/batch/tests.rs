use super::*;
use crate::{
    app::discovery::{MediaToolsDiscovery, MediaToolsState},
    media::{model::StreamKind, probe::parse_ffprobe_json, tools::MediaTools},
};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
fn temp_file(label: &str) -> (PathBuf, PathBuf) {
    let directory = loop {
        let directory = std::env::temp_dir().join(format!(
            "apollo-batch-{label}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&directory) {
            Ok(()) => break directory,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("could not create fixture: {error}"),
        }
    };
    let path = directory.join("recording.mkv");
    fs::write(&path, b"media").unwrap();
    (directory, path)
}
fn media(path: &Path) -> MediaFile {
    parse_ffprobe_json(
        path,
        include_bytes!("../../../tests/fixtures/ffprobe/one_audio.json"),
    )
    .unwrap()
}
fn ready_batch_source(media: MediaFile) -> BatchSourceState {
    BatchSourceState::Ready {
        source_identity: SourceIdentity::read(&media.path).unwrap(),
        media,
    }
}
fn app() -> Application {
    Application {
        discovery: MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
            "missing-ffmpeg",
            "missing-ffprobe",
        ))),
        ..Application::default()
    }
}
fn assert_no_batch_execution(app: &Application) {
    assert!(app.queue.scheduler().jobs().is_empty());
    assert!(!app.queue.scheduler().is_running());
}
#[test]
fn audio_extract_batch_requires_explicit_selection_for_multi_track_sources() {
    let (first_directory, first_path) = temp_file("batch-audio-first");
    let (second_directory, second_path) = temp_file("batch-audio-second");
    let first_media = media(&first_path);
    let second_media = parse_ffprobe_json(
        &second_path,
        include_bytes!("../../../tests/fixtures/ffprobe/multiple_audio.json"),
    )
    .unwrap();
    let mut batch = BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::AudioExtract,
        defaults: Some(BatchDefaults::AudioExtract {
            format: AudioExtractFormat::Original,
        }),
        items: vec![
            BatchItem {
                id: BatchItemId(0),
                path: first_path,
                included: true,
                source: ready_batch_source(first_media),
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            },
            BatchItem {
                id: BatchItemId(1),
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
    };

    refresh_batch_items(&mut batch, None);

    assert_eq!(batch.items[0].readiness, BatchReadiness::Ready);
    assert_eq!(batch.items[1].readiness, BatchReadiness::NeedsAttention);
    assert_eq!(
        batch.items[1].reason.as_deref(),
        Some("Select one audio track.")
    );
    assert!(batch.items[0].operation.is_some());
    assert!(batch.items[1].operation.is_none());
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
                id: BatchItemId(0),
                path: first_path,
                included: false,
                source: ready_batch_source(first_media),
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            },
            BatchItem {
                id: BatchItemId(1),
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
    });
    app.show_batch_setup();
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
            id: BatchItemId(0),
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
    });
    app.show_batch_setup();

    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(0))));
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

    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(0))));
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
                id: BatchItemId(0),
                path: first_path,
                included: true,
                source: ready_batch_source(first_media),
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            },
            BatchItem {
                id: BatchItemId(1),
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
    });
    app.show_batch_setup();

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

const TIMEOUT: Duration = Duration::from_secs(10);

fn inspected(path: &Path) -> Result<crate::media::batch::BatchInspectedMedia, String> {
    Ok(crate::media::batch::BatchInspectedMedia {
        source_identity: SourceIdentity::read(path).map_err(|error| error.to_string())?,
        media: media(path),
    })
}

fn settle_inspections(app: &mut Application, wakes: &mpsc::Receiver<()>) {
    let deadline = Instant::now() + TIMEOUT;
    while app.batch_inspecting() {
        app.poll_batch_inspection();
        if app.batch_inspecting() {
            wakes
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
        }
    }
}

#[test]
fn queue_round_trip_preserves_batch_inspection_and_return_origin() {
    let (first_dir, first) = temp_file("queue-batch-first");
    let (second_dir, second) = temp_file("queue-batch-second");
    let mut app = app();
    app.enter_workspace(Operation::Remux);
    let (wake_tx, wake_rx) = mpsc::channel();
    assert!(
        app.start_batch_paths_with(vec![first.clone(), second.clone()], |paths| {
            BatchInspectionOwner::spawn_with(
                paths,
                move || {
                    let _ = wake_tx.send(());
                },
                inspected,
            )
        })
    );
    app.enter_queue();
    assert_eq!(
        app.queue_origin(),
        Some(crate::app::QueueOrigin::BatchSetup)
    );
    assert!(app.batch_inspecting());
    assert!(app.batch_setup.is_some());
    assert!(!app.start_batch_paths_with(vec![first, second], |_| panic!(
        "must retain the existing batch"
    )));
    settle_inspections(&mut app, &wake_rx);
    let retained = app.batch_setup.clone().unwrap();
    assert!(
        retained
            .items
            .iter()
            .all(|item| matches!(item.source, BatchSourceState::Ready { .. }))
    );
    app.return_from_queue();
    assert_eq!(app.navigation(), crate::app::NavigationState::BatchSetup);
    assert_eq!(app.batch_setup.as_ref(), Some(&retained));
    assert!(app.finish_batch_setup());
    assert_eq!(app.active_operation(), Some(Operation::Remux));
    assert!(app.batch_setup.is_none());
    fs::remove_dir_all(first_dir).unwrap();
    fs::remove_dir_all(second_dir).unwrap();
}

#[test]
fn inspection_append_and_atomic_enqueue_preserve_item_identity() {
    let (first_dir, first) = temp_file("inspection-first");
    let (failed_dir, failed) = temp_file("inspection-failed");
    let (last_dir, last) = temp_file("inspection-append");
    let mut app = app();
    app.enter_workspace(Operation::Remux);
    let (wake_tx, wake_rx) = mpsc::channel();
    let first_identity = SourceIdentity::read(&first).unwrap();
    let failed_path = failed.clone();
    let (permit_tx, permit_rx) = mpsc::channel();
    let permit_rx = std::sync::Mutex::new(permit_rx);
    assert!(
        app.start_batch_paths_with(vec![first.clone(), failed.clone()], |paths| {
            let wake = wake_tx.clone();
            BatchInspectionOwner::spawn_with(
                paths,
                move || {
                    let _ = wake.send(());
                },
                move |path| {
                    if path == failed_path {
                        return Err("invalid media fixture".to_owned());
                    }
                    permit_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(TIMEOUT)
                        .map_err(|error| error.to_string())?;
                    inspected(path)
                },
            )
        })
    );
    // Reordering the view's backing rows must not redirect a worker result or item action.
    app.batch_setup.as_mut().unwrap().items.swap(0, 1);
    wake_rx.recv_timeout(TIMEOUT).unwrap();
    app.poll_batch_inspection();
    let batch = app.batch_setup.as_ref().unwrap();
    assert_eq!(
        app.batch_availability(),
        Some(BatchAvailability {
            can_configure: false,
            can_add_files: false,
            can_enqueue: false
        })
    );
    assert_eq!(batch.items[0].id, BatchItemId(1));
    assert!(matches!(batch.items[0].source, BatchSourceState::Failed));
    assert!(!batch.items[0].included);
    assert!(matches!(
        batch.items[1].source,
        BatchSourceState::Inspecting
    ));
    assert!(!app.enqueue_batch());
    assert_no_batch_execution(&app);
    permit_tx.send(()).unwrap();
    settle_inspections(&mut app, &wake_rx);
    assert!(matches!(&app.batch_setup.as_ref().unwrap().items[1].source,
        BatchSourceState::Ready { source_identity, media } if source_identity == &first_identity && media.path == first));
    app.select_batch_operation(Operation::Remux);
    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(0))));
    let output = first_dir.join("shared.mkv");
    app.batch_edit
        .as_mut()
        .unwrap()
        .remux
        .set_custom_output_path(output.clone());
    assert!(app.save_batch_edit());
    let retained = app.batch_setup.as_ref().unwrap().items[1].clone();
    let alias = first_dir.join(".").join(first.file_name().unwrap());
    assert!(
        app.append_batch_paths_with(vec![alias, last.clone(), last.clone()], |paths| {
            assert_eq!(
                paths,
                vec![
                    first_dir.join(".").join(first.file_name().unwrap()),
                    last.clone(),
                    last.clone()
                ]
            );
            let wake = wake_tx.clone();
            BatchInspectionOwner::spawn_with(
                paths,
                move || {
                    let _ = wake.send(());
                },
                inspected,
            )
        })
    );
    settle_inspections(&mut app, &wake_rx);
    let batch = app.batch_setup.as_ref().unwrap();
    assert_eq!(batch.items.len(), 5);
    assert_eq!(batch.items[1], retained);
    assert_eq!(batch.items[2].id, BatchItemId(2));
    assert!(!app.batch_availability().unwrap().can_enqueue);
    assert_eq!(batch.items[4].readiness, BatchReadiness::NeedsAttention);
    assert!(!app.enqueue_batch());
    assert_no_batch_execution(&app);
    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(2))));
    app.batch_edit
        .as_mut()
        .unwrap()
        .remux
        .set_custom_output_path(output.clone());
    assert!(app.save_batch_edit());
    assert!(!app.batch_availability().unwrap().can_enqueue);
    assert_eq!(
        app.batch_setup.as_ref().unwrap().items[2].readiness,
        BatchReadiness::NeedsAttention
    );
    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(2))));
    let alias_output = first_dir.join("alias-output.mkv");
    app.batch_edit
        .as_mut()
        .unwrap()
        .remux
        .set_custom_output_path(alias_output.clone());
    assert!(app.save_batch_edit());
    app.toggle_batch_item(BatchItemId(4));
    assert!(app.batch_availability().unwrap().can_enqueue);
    app.toggle_batch_item(BatchItemId(4));
    assert!(!app.batch_availability().unwrap().can_enqueue);
    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(4))));
    app.batch_edit
        .as_mut()
        .unwrap()
        .remux
        .set_custom_output_path(last_dir.join("second-output.mkv"));
    assert!(app.save_batch_edit());
    assert!(app.batch_availability().unwrap().can_enqueue);
    assert!(app.enqueue_batch());
    assert!(app.batch_setup.is_none());
    assert!(!app.enqueue_batch());
    let jobs = app.queue.scheduler().jobs();
    assert_eq!(jobs.len(), 4);
    assert_eq!(
        jobs.iter().map(|job| job.id().get()).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(jobs[0].operation().input(), first);
    assert_eq!(jobs[0].operation().output(), output);
    assert_eq!(jobs[1].operation().output(), alias_output);
    assert_eq!(jobs[2].operation().input(), last);
    assert_eq!(jobs[3].operation().input(), last);
    assert_eq!(
        app.queue.scheduler().run_state(),
        crate::queue::QueueRunState::Stopped
    );
    app.return_from_queue();
    assert_eq!(app.active_operation(), Some(Operation::Remux));
    for directory in [first_dir, failed_dir, last_dir] {
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn leaving_batch_retires_inspection_before_a_replacement_batch() {
    let (old_dir, old) = temp_file("retired");
    let (new_dir, new) = temp_file("replacement");
    let mut app = app();
    let (started_tx, started_rx) = mpsc::channel();
    let (permit_tx, permit_rx) = mpsc::channel();
    let permit_rx = std::sync::Mutex::new(permit_rx);
    let (finished_tx, finished_rx) = mpsc::channel();
    assert!(
        app.start_batch_paths_with(vec![old.clone(), old.clone()], |paths| {
            BatchInspectionOwner::spawn_with(
                paths,
                || {},
                move |path| {
                    started_tx.send(()).unwrap();
                    permit_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(TIMEOUT)
                        .map_err(|error| error.to_string())?;
                    let result = inspected(path);
                    let _ = finished_tx.send(());
                    result
                },
            )
        })
    );
    for _ in 0..2 {
        started_rx.recv_timeout(TIMEOUT).unwrap();
    }
    // A successful single-source replacement exits batch setup, not just its explicit Back action.
    app.enter_workspace(Operation::AudioMix);
    assert!(!app.batch_inspecting());
    assert!(app.batch_setup.is_none());
    let (wake_tx, wake_rx) = mpsc::channel();
    assert!(
        app.start_batch_paths_with(vec![new.clone(), new.clone()], |paths| {
            BatchInspectionOwner::spawn_with(
                paths,
                move || {
                    let _ = wake_tx.send(());
                },
                inspected,
            )
        })
    );
    for _ in 0..2 {
        permit_tx.send(()).unwrap();
    }
    for _ in 0..2 {
        finished_rx.recv_timeout(TIMEOUT).unwrap();
    }
    settle_inspections(&mut app, &wake_rx);
    assert!(app.batch_setup.as_ref().unwrap().items.iter().all(|item| {
        matches!(&item.source, BatchSourceState::Ready { media, .. } if media.path == new)
    }));
    assert!(app.finish_batch_setup());
    assert_eq!(app.active_operation(), Some(Operation::AudioMix));
    fs::remove_dir_all(old_dir).unwrap();
    fs::remove_dir_all(new_dir).unwrap();
}

#[test]
fn stale_batch_admission_marks_only_the_originating_item_and_retains_other_requests() {
    let (first_dir, first_path) = temp_file("admission-first");
    let (second_dir, second_path) = temp_file("admission-second");
    let mut app = app();
    let items = [
        (&first_path, BatchItemId(7)),
        (&second_path, BatchItemId(12)),
    ]
    .into_iter()
    .map(|(path, id)| BatchItem {
        id,
        path: path.clone(),
        included: true,
        source: ready_batch_source(media(path)),
        operation: None,
        readiness: BatchReadiness::Inspecting,
        reason: None,
        overridden: false,
    })
    .collect();
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::Remux,
        defaults: Some(BatchDefaults::Remux {
            container: MediaContainer::Matroska,
        }),
        items,
        error: None,
        append_error: None,
    });
    app.show_batch_setup();
    refresh_batch_items(app.batch_setup.as_mut().unwrap(), None);
    let first = app.batch_setup.as_ref().unwrap().items[0].clone();
    assert_eq!(first.readiness, BatchReadiness::Ready);
    fs::write(&second_path, b"changed source after inspection").unwrap();
    assert!(!app.enqueue_batch());
    assert_no_batch_execution(&app);
    assert_eq!(app.navigation(), NavigationState::BatchSetup);
    let batch = app.batch_setup.as_ref().unwrap();
    assert_eq!(batch.items[0], first);
    assert_eq!(batch.items[1].id, BatchItemId(12));
    assert_eq!(batch.items[1].readiness, BatchReadiness::NeedsAttention);
    assert!(
        batch.items[1]
            .reason
            .as_deref()
            .unwrap()
            .contains("source changed")
    );
    app.toggle_batch_item(BatchItemId(12));
    assert!(app.enqueue_batch());
    let jobs = app.queue.scheduler().jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id().get(), 1);
    assert_eq!(jobs[0].operation(), first.operation.as_ref().unwrap());
    fs::remove_dir_all(first_dir).unwrap();
    fs::remove_dir_all(second_dir).unwrap();
}

#[test]
fn batch_readiness_recovers_after_pending_destination_is_removed_in_queue() {
    let (directory, path) = temp_file("reservation-release");
    let source = media(&path);
    let mut app = app();
    let identity = SourceIdentity::read(&path).unwrap();
    let editor = OperationEditorState::new(0, None, source.clone(), Operation::Remux);
    let request = editor.validated_request(None).unwrap();
    let pending = app
        .queue
        .enqueue(&identity, &source, request.clone())
        .unwrap();
    app.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::Remux,
        defaults: batch_defaults_from_editor(&editor),
        items: vec![BatchItem {
            id: BatchItemId(6),
            path: path.clone(),
            included: true,
            source: ready_batch_source(source),
            operation: Some(request.clone()),
            readiness: BatchReadiness::Ready,
            reason: None,
            overridden: false,
        }],
        error: None,
        append_error: None,
    });
    app.show_batch_setup();
    assert!(!app.batch_availability().unwrap().can_enqueue);
    assert!(
        app.batch_setup.as_ref().unwrap().items[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("reserved")
    );
    assert!(app.enter_queue());
    assert!(matches!(
        app.queue_action(crate::app::QueueAction::Remove(pending), Default::default),
        crate::app::queue::QueueEffect::Handled(true)
    ));
    app.return_from_queue();
    assert!(app.batch_availability().unwrap().can_enqueue);
    assert!(app.batch_setup.as_ref().unwrap().items[0].reason.is_none());
    assert!(app.enqueue_batch());
    assert_eq!(app.queue.scheduler().jobs()[0].id().get(), 2);
    assert_eq!(app.queue.scheduler().jobs()[0].operation(), &request);
    fs::remove_dir_all(directory).unwrap();
}

use super::*;

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
    let decode = FramePreviewDecode::spawn_with(7, request, wake, move || {
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
        let editor = &mut editor.state;
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
    assert!(!app.core.frame_work.frame_index_query.cancel());
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
    app.core.frame_work.frame_index_query.cancel();
    app.settle_frame_index_request(
        app.state.frame_capture.as_ref().unwrap().session_id,
        request,
        Err(FrameIndexError::Cancelled),
    );

    let session = app.state.frame_capture.as_ref().unwrap();
    assert!(session.pending_step.is_none());
    assert!(matches!(
        session.frame_navigator.state(),
        FrameIndexState::Failed {
            request,
            error: FrameIndexError::Cancelled,
        } if request.center == edge.into()
    ));
    assert!(!app.core.frame_work.frame_index_query.cancel());
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
    let editor = &mut editor.state;
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
    assert!(
        app.core
            .frame_work
            .frame_preview_extraction
            .poll()
            .is_none()
    );
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
    let editor = &mut editor.state;
    let session_id = editor.session_id;
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
    app.core.frame_work.frame_preview_decode = Some(FramePreviewDecode {
        session_id,
        request: stale_request,
        receiver,
    });
    sender
        .send(Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap()))
        .unwrap();

    app.poll_frame_preview_decode();

    assert!(app.core.frame_work.frame_preview_decode.is_none());
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
    app.core.frame_work.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
        session_id,
        request_id: request.request_id(),
        receiver,
    });

    sender.send(Ok(thumbnail())).unwrap();
    app.poll_frame_capture_thumbnail();

    assert!(app.core.frame_work.frame_capture_thumbnail_load.is_none());
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
fn batch_frame_capture_defaults_stay_simple_while_item_override_owns_precision_support() {
    let (directory, path) = temp_file("batch-frame-capture-preview");
    let source = media(&path);
    let request = FrameCaptureDraft::new(&source).request(&source).unwrap();
    let mut app = app();
    app.core.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::FrameCapture,
        defaults: Some(BatchDefaults::FrameCapture {
            timestamp: FrameTimestamp::ZERO,
            format: FrameCaptureFormat::Png,
        }),
        items: vec![BatchItem {
            id: BatchItemId(0),
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
    });
    app.core.show_batch_setup();
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    assert!(app.begin_batch_edit(BatchEditTarget::Defaults));
    let defaults = app.core.batch_edit.as_ref().unwrap();
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

    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(0))));
    let item_edit = app.core.batch_edit.as_ref().unwrap();
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
        .core
        .batch_edit
        .as_ref()
        .unwrap()
        .frame_capture
        .valid_position(&app.core.batch_edit.as_ref().unwrap().media)
        .unwrap();
    assert!(app.save_batch_edit());
    let Some(QueuedOperation::FrameCapture(saved)) = app.core.batch_setup.as_ref().unwrap().items
        [0]
    .operation
    .as_ref() else {
        panic!("batch item must save a Frame Capture request");
    };
    assert_eq!(saved.position, selected);
    assert!(!app.core.source.is_inspecting());
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
fn queue_frame_capture_edit_reuses_precision_support_and_saves_exact_position() {
    let (directory, path) = temp_file("queue-frame-capture-preview");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let job_id = app
        .enqueue_current_operation(Operation::FrameCapture)
        .unwrap()
        .unwrap();
    let queued_identity = app.core.queue.scheduler().jobs()[0]
        .retained_intake()
        .current_with_identity()
        .unwrap()
        .0
        .clone();
    app.enter_queue();

    assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
    let edit = app.core.queue_edit().unwrap();
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
        .core
        .queue_edit()
        .unwrap()
        .frame_capture
        .valid_position(&app.core.queue_edit().unwrap().media)
        .unwrap();
    assert_eq!(
        selected,
        FramePosition::from(
            ResolvedFramePosition::new(170, FrameTimeBase::new(1, 1_000).unwrap()).unwrap()
        )
    );

    assert!(app.save_queue_edit());
    let QueuedOperation::FrameCapture(saved) = app.core.queue.scheduler().jobs()[0].operation()
    else {
        panic!("frame capture edit must remain Frame Capture");
    };
    assert_eq!(saved.position, selected);
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), job_id);
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
    let session_id = app.core.queue_edit().unwrap().session_id;
    let stale_request = app
        .core
        .queue_edit_mut()
        .unwrap()
        .frame_capture
        .preview
        .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
        .unwrap();
    assert!(app.select_queue_edit_tool(Operation::Remux));
    let edit = app.core.queue_edit().unwrap();
    assert!(matches!(
        edit.frame_capture.preview.state(),
        PreviewState::Idle
    ));
    assert!(
        app.queue_editor_presentation
            .as_ref()
            .unwrap()
            .preview_texture
            .is_none()
    );
    assert!(!app.settle_frame_preview(
        session_id,
        stale_request,
        Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
    ));

    assert!(app.select_queue_edit_tool(Operation::FrameCapture));
    let current_request = match app.core.queue_edit().unwrap().frame_capture.preview.state() {
        PreviewState::Debouncing { request, .. } => request.clone(),
        state => panic!("expected fresh editor preview, got {state:?}"),
    };
    assert!(app.finish_queue_edit());
    assert!(!app.settle_frame_preview(
        session_id,
        current_request,
        Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
    ));
    assert!(!app.core.source.is_inspecting());
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
    let editor = &mut editor.state;
    editor.selection.set_position(media, resolved.into());
    editor.selection.set_output_filename_stem(OsString::new());
    assert!(!editor.selection.can_capture(media));
    assert!(copy_frame_action_eligible(media, resolved.into()));

    app.handle_frame_capture_action(FrameCaptureSelectionAction::CopyRequested)
        .unwrap();

    assert!(app.core.queue.scheduler().jobs().is_empty());
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
    assert!(!app.core.frame_work.copy_frame_extraction.is_active());
    assert!(app.core.frame_work.copy_frame_worker.is_none());
    assert!(app.core.queue.scheduler().jobs().is_empty());
    assert!(app.core.frame_work.frame_capture_thumbnail_load.is_none());
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

#[test]
fn returning_from_queue_restarts_retired_frame_work_without_losing_selection() {
    let (directory, path) = temp_file("frame-queue-return");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let position = session.selection.valid_position(&session.media).unwrap();
    let previous = session
        .preview
        .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
        .unwrap();
    let session_id = session.session_id;
    app.enter_queue();
    assert!(app.handle_queue_action(QueueAction::Back));
    let session = app.state.frame_capture.as_mut().unwrap();
    assert_eq!(session.session_id, session_id);
    assert_eq!(
        session.selection.valid_position(&session.media),
        Some(position)
    );
    let current = session
        .preview
        .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
        .unwrap();
    assert!(!app.settle_frame_preview(
        session_id,
        previous,
        Ok(PreviewImage::from_rgba([1, 1], vec![0; 4]).unwrap())
    ));
    assert!(app.settle_frame_preview(
        session_id,
        current,
        Ok(PreviewImage::from_rgba([1, 1], vec![255; 4]).unwrap())
    ));
    fs::remove_dir_all(directory).unwrap();
}

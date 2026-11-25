use super::*;

#[test]
fn inspection_completion_enters_default_source_workspace() {
    let (directory, path) = temp_file("inspection-mode-switch");
    let mut app = app();
    let sender = begin_pending_inspection(&mut app, &path);

    sender.send(Ok(media(&path))).unwrap();
    app.poll_media_inspection();

    assert!(!app.core.source.is_inspecting());
    assert_workspace(&app.core, Operation::AudioMix);
    assert!(app.state.audio_mix.is_some());
    assert_eq!(
        app.state
            .audio_mix
            .as_ref()
            .map(|session| &session.media.path),
        Some(&path)
    );
    assert_eq!(
        app.core.source.current().map(|media| &media.path),
        Some(&path)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn source_replacement_clears_thumbnail_and_rejects_stale_completion() {
    let (first_directory, first_path) = temp_file("thumbnail-first-source");
    let (second_directory, second_path) = temp_file("thumbnail-second-source");
    let mut app = app();
    app.load_media_path_with_probe(&first_path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let old_session = app.state.frame_capture.as_mut().unwrap();
    let old_session_id = old_session.session_id;
    let old_request = old_session
        .lifecycle_ui
        .begin_thumbnail(Path::new("old-capture.png"))
        .unwrap();

    app.load_media_path_with_probe(&second_path, |path| Ok(media(path)));
    assert!(app.core.frame_work.frame_capture_thumbnail_load.is_none());
    let replacement = app.state.frame_capture.as_ref().unwrap();
    assert_ne!(replacement.session_id, old_session_id);
    assert!(matches!(
        replacement.lifecycle_ui.thumbnail(),
        FrameCaptureThumbnailState::Empty
    ));

    let (sender, receiver) = mpsc::sync_channel(1);
    app.core.frame_work.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
        session_id: old_session_id,
        request_id: old_request.request_id(),
        receiver,
    });
    sender.send(Ok(thumbnail())).unwrap();
    app.poll_frame_capture_thumbnail();
    assert!(matches!(
        app.state
            .frame_capture
            .as_ref()
            .unwrap()
            .lifecycle_ui
            .thumbnail(),
        FrameCaptureThumbnailState::Empty
    ));

    fs::remove_dir_all(first_directory).unwrap();
    fs::remove_dir_all(second_directory).unwrap();
}

#[test]
fn close_source_releases_thumbnail_worker_and_session_state() {
    let (directory, path) = temp_file("thumbnail-close-source");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    let session = app.state.frame_capture.as_mut().unwrap();
    let request = session
        .lifecycle_ui
        .begin_thumbnail(Path::new("capture.png"))
        .unwrap();
    let (_sender, receiver) = mpsc::sync_channel(1);
    app.core.frame_work.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
        session_id: session.session_id,
        request_id: request.request_id(),
        receiver,
    });

    app.handle_source_action(SourceAction::CloseSource);

    assert!(app.core.frame_work.frame_capture_thumbnail_load.is_none());
    assert!(app.state.frame_capture.is_none());
    assert_no_source(&app.core);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_inspection_returns_to_intake_without_current_metadata() {
    let (directory, path) = temp_file("inspection-failure");
    let mut app = app();
    let sender = begin_pending_inspection(&mut app, &path);

    sender
        .send(Err(MediaProbeError::FfprobeFailed {
            code: Some(1),
            diagnostic: "probe failed".to_owned(),
        }))
        .unwrap();
    app.poll_media_inspection();

    assert!(!app.core.source.is_inspecting());
    assert_no_source(&app.core);
    assert!(app.core.source.current().is_none());
    assert!(app.state.audio_mix.is_none());
    assert!(app.state.upscale.is_none());
    assert_eq!(
        app.core.source.error().map(MediaError::kind),
        Some(MediaErrorKind::ProbeFailure)
    );

    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    let mut output = context.run_ui(Default::default(), |ui| {
        app.show_navigation(ui);
    });
    output.textures_delta.clear();
    assert!(output_contains_text(
        &output,
        "This file could not be inspected."
    ));
    assert!(output_contains_text(&output, "Open files"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn accepted_source_presentation_is_reused_until_source_replacement() {
    let (directory, first) = temp_file("source-presentation");
    let second = directory.join("replacement.mkv");
    fs::write(&second, b"replacement").unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&first, |path| Ok(media(path)));
    let original_presentation = app.state.source_presentation.as_ref().unwrap().clone();
    let original_id = app.core.source.session_id().unwrap();

    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    for operation in [
        Operation::AudioMix,
        Operation::VideoUpscale,
        Operation::Remux,
        Operation::AudioExtract,
        Operation::FrameCapture,
    ] {
        app.select_tool(operation);
        let draft_id = match operation {
            Operation::AudioMix => app.state.audio_mix.as_ref().unwrap().session_id,
            Operation::VideoUpscale => app.state.upscale.as_ref().unwrap().session_id,
            Operation::Remux => app.state.remux.as_ref().unwrap().session_id,
            Operation::AudioExtract => app.state.audio_extract.as_ref().unwrap().session_id,
            Operation::FrameCapture => app.state.frame_capture.as_ref().unwrap().session_id,
        };
        assert_eq!(draft_id, original_id);
        let output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
        output.drop_without_applying_deltas();
        assert!(Arc::ptr_eq(
            app.state.source_presentation.as_ref().unwrap(),
            &original_presentation
        ));
    }

    app.load_media_path_with_probe(&first, |_| {
        panic!("unchanged source must not be probed again")
    });
    assert_eq!(app.core.source.session_id(), Some(original_id));
    assert!(Arc::ptr_eq(
        app.state.source_presentation.as_ref().unwrap(),
        &original_presentation
    ));

    app.load_media_path_with_probe(&second, |path| Ok(media(path)));
    assert_ne!(app.core.source.session_id(), Some(original_id));
    assert!(!Arc::ptr_eq(
        app.state.source_presentation.as_ref().unwrap(),
        &original_presentation
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn change_file_cancel_preserves_complete_workspace() {
    let (directory, path) = temp_file("change-cancel");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::FrameCapture);
    app.state
        .frame_capture
        .as_mut()
        .unwrap()
        .selection
        .set_format(FrameCaptureFormat::Jpeg);
    let before = app.state.clone();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::MediaInput,
        outcome: DialogOutcome::Cancelled,
    });

    assert_eq!(app.state, before);
    assert!(app.core.source.error().is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn change_file_keeps_workspace_until_replacement_probe_publishes() {
    let (directory, first) = temp_file("change-transaction");
    let second = directory.join("replacement.mkv");
    fs::write(&second, b"replacement").unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&first, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_container(MediaContainer::Mp4);
    app.core
        .source
        .set_error(MediaError::probe_failure("previous replacement failed"));
    let old_session = app.state.remux.clone().unwrap();

    let request = app
        .begin_media_path(&second)
        .expect("replacement should require inspection");

    assert_workspace(&app.core, Operation::Remux);
    assert!(app.core.source.error().is_none());
    assert_eq!(app.state.remux.as_ref(), Some(&old_session));
    assert_eq!(
        app.core.source.current().map(|media| &media.path),
        Some(&first)
    );

    app.complete_media_probe(request, Ok(media(&second)));

    assert_workspace(&app.core, Operation::Remux);
    let replacement = app.state.remux.as_ref().unwrap();
    assert_ne!(replacement.session_id, old_session.session_id);
    assert_eq!(replacement.media.path, second);
    assert_eq!(replacement.selection.container(), MediaContainer::Matroska);
    assert!(app.core.source.error().is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn open_valid_file_routes_audio_mix_to_ready_state() {
    let (directory, path) = temp_file("open");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));

    assert_workspace(&app.core, Operation::AudioMix);
    assert_eq!(
        app.state.audio_mix.as_ref().map(|s| &s.media.path),
        Some(&path)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dropped_valid_file_enters_default_workspace() {
    let (directory, path) = temp_file("drop");
    let mut app = app();
    assert!(file_intake_enabled(
        app.core.upscale_executor.state(),
        app.core.audio_extract_executor.state(),
        app.core.frame_capture_executor.state(),
    ));
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));

    assert_workspace(&app.core, Operation::AudioMix);
    assert_eq!(
        app.state.audio_mix.as_ref().map(|s| &s.media.path),
        Some(&path)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn one_file_drop_starts_normal_media_inspection() {
    let (directory, path) = temp_file("single-drop");
    let mut app = app();

    handle_drop(&mut app, &[&path]);

    assert_eq!(app.core.source.inspection_path(), Some(path.as_path()));
    assert!(app.core.source.error().is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn multiple_file_drop_enters_batch_setup_in_source_order() {
    let (first_directory, first) = temp_file("multi-drop-first");
    let (second_directory, second) = temp_file("multi-drop-second");
    let mut app = app();

    handle_drop(&mut app, &[&first, &second]);

    assert!(!app.core.source.is_inspecting());
    assert!(app.core.source.current().is_none());
    assert_eq!(app.core.navigation(), NavigationState::BatchSetup);
    let batch = app.core.batch_setup.as_ref().unwrap();
    assert_eq!(
        batch
            .items
            .iter()
            .map(|item| &item.path)
            .collect::<Vec<_>>(),
        vec![&first, &second]
    );
    fs::remove_dir_all(first_directory).unwrap();
    fs::remove_dir_all(second_directory).unwrap();
}

#[test]
fn directory_drop_uses_shared_intake_error_path() {
    let directory = std::env::temp_dir().join(format!(
        "apollo-phase49-directory-drop-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).unwrap();
    let mut app = app();

    handle_drop(&mut app, &[&directory]);

    assert!(!app.core.source.is_inspecting());
    assert_eq!(
        app.core.source.error().map(MediaError::kind),
        Some(MediaErrorKind::SourceUnavailable)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn missing_path_drop_uses_shared_intake_error_path() {
    let directory = std::env::temp_dir().join(format!(
        "apollo-phase49-missing-drop-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    let missing = directory.join("missing.mkv");
    let mut app = app();

    handle_drop(&mut app, &[&missing]);

    assert!(!app.core.source.is_inspecting());
    assert_eq!(
        app.core.source.error().map(MediaError::kind),
        Some(MediaErrorKind::SourceUnavailable)
    );
}

#[test]
fn open_dialog_selected_path_starts_the_shared_intake_route() {
    let (directory, path) = temp_file("dialog-open-route");
    let mut app = app();

    let (wake_tx, wake_rx) = mpsc::channel();
    let selected = path.clone();
    app.file_dialog = FileDialogOwner::new_for_test(
        move || {
            let _ = wake_tx.send(());
        },
        move || Ok(move |_: &FileDialogRequest| Ok(Some(selected.clone()))),
    );
    app.start_file_dialog(FileDialogRequest::OpenSource {
        initial_directory: None,
    });
    wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    app.poll_file_dialog();
    assert_eq!(app.core.source.inspection_path(), Some(path.as_path()));
    assert_eq!(
        app.core.settings.source_directory(),
        Some(directory.clone())
    );
    assert!(!app.file_dialog.is_pending());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn open_dialog_failure_surfaces_bounded_intake_feedback() {
    let mut app = app();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::MediaInput,
        outcome: DialogOutcome::Failed("native open diagnostic marker".to_owned()),
    });

    let error = app
        .core
        .source
        .error()
        .expect("open dialog failure should become intake state");
    assert_eq!(error.kind(), MediaErrorKind::FilePickerUnavailable);
    assert_eq!(error.diagnostic(), Some("native open diagnostic marker"));
    assert!(!app.state.intake_error_details_open);
}

#[test]
fn change_file_probe_failure_is_rendered_and_survives_operation_switching() {
    let (directory, first) = temp_file("stale-first");
    let second = directory.join("replacement.mkv");
    fs::write(&second, b"replacement").unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&first, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);
    let before = app.state.clone();
    app.load_media_path_with_probe(&second, |_| {
        Err(crate::media::probe::MediaProbeError::FfprobeFailed {
            code: Some(1),
            diagnostic: "probe failed".to_owned(),
        })
    });

    assert_workspace(&app.core, Operation::VideoUpscale);
    assert_eq!(app.state.upscale, before.upscale);
    assert_eq!(
        app.core.source.current().map(|media| &media.path),
        Some(&first)
    );
    assert_eq!(
        app.core.source.error().map(MediaError::kind),
        Some(MediaErrorKind::ProbeFailure)
    );

    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    let mut output = context.run_ui(Default::default(), |ui| {
        app.show_navigation(ui);
    });
    output.textures_delta.clear();
    assert!(output_contains_text(
        &output,
        "This file could not be inspected."
    ));
    assert!(output_contains_text(
        &output,
        "Check that the source is a readable media file and try again."
    ));
    assert!(output_contains_text(&output, "Upscale to"));
    assert!(output_contains_text(&output, "Change file"));
    assert!(output_contains_text(&output, "Audio Mix"));

    app.select_tool(Operation::Remux);

    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(app.state.upscale, before.upscale);
    assert_eq!(
        app.core.source.error().map(MediaError::kind),
        Some(MediaErrorKind::ProbeFailure)
    );
    let mut output = context.run_ui(Default::default(), |ui| {
        app.show_navigation(ui);
    });
    output.textures_delta.clear();
    assert!(output_contains_text(
        &output,
        "This file could not be inspected."
    ));
    assert!(output_contains_text(&output, "Remux"));
    assert!(output_contains_text(&output, "Change file"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn replacement_source_retires_preserved_ready_sessions() {
    let (directory, first) = temp_file("replace-after-back");
    let second = directory.join("replacement.mkv");
    fs::write(&second, b"replacement").unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&first, |path| Ok(media(path)));
    app.select_tool(Operation::VideoUpscale);
    app.select_tool(Operation::AudioMix);
    assert!(app.state.audio_mix.is_some());
    let old_audio_mix = app.state.audio_mix.as_mut().unwrap();
    old_audio_mix.selection.set_normalization_enabled(true);
    assert!(
        old_audio_mix
            .selection
            .set_normalization_targets(LoudnessTargets::from_tenths(-220, -9, 135))
    );
    app.select_tool(Operation::VideoUpscale);
    let old_output = directory.join("old-custom.mp4");
    app.state
        .upscale
        .as_mut()
        .expect("upscale session should exist")
        .selection
        .set_custom_output_path(old_output);
    app.load_media_path_with_probe(&second, |path| Ok(media(path)));

    assert!(app.state.audio_mix.is_none());
    let session = app
        .state
        .upscale
        .as_ref()
        .expect("replacement should create a new Upscale session");
    assert_workspace(&app.core, Operation::VideoUpscale);
    assert_eq!(session.media.path, second);
    assert!(!session.selection.output_is_customized());
    assert_eq!(session.selection.target(), UpscaleTarget::Uhd4k);
    assert_eq!(session.selection.custom_boundary(), None);
    assert_eq!(session.selection.codec(), VideoCodec::H264);
    assert_eq!(
        app.core.source.current().map(|media| &media.path),
        Some(&second)
    );

    app.select_tool(Operation::AudioMix);
    let audio_mix = app
        .state
        .audio_mix
        .as_ref()
        .expect("replacement should create a new Audio Mix session");
    assert!(
        audio_mix
            .selection
            .request(&audio_mix.media)
            .selected_tracks
            .iter()
            .all(|track| track.gain == TrackGain::ZERO)
    );
    assert!(!audio_mix.selection.normalization_enabled());
    assert_eq!(
        audio_mix.selection.normalization_targets(),
        LoudnessTargets::default()
    );
    fs::remove_dir_all(directory).unwrap();
}

use super::super::dialogs::editor_output_dialog_request;
use super::*;
use crate::app::dialogs::OutputDraft;
use crate::file_dialog::FileDialogStartError;

#[test]
fn dialog_requests_prefer_independent_remembered_directories() {
    let (source_directory, source_path) = temp_file("remember-source");
    let output_directory = source_directory.join("remembered-output");
    fs::create_dir_all(&output_directory).unwrap();
    let mut app = app();
    app.core
        .settings
        .remember_source(&source_directory.join("previous-source.mp4"));
    app.core
        .settings
        .remember_output(&output_directory.join("previous-output.mkv"));

    let open = app
        .core
        .prepare_file_dialog_request(FileDialogRequest::OpenSources {
            initial_directory: None,
            target: DialogTarget::BatchInput,
        });
    assert_eq!(
        open,
        FileDialogRequest::OpenSources {
            initial_directory: Some(source_directory.clone()),
            target: DialogTarget::BatchInput,
        }
    );

    app.load_media_path_with_probe(&source_path, |path| Ok(media(path)));
    let session = app.state.audio_mix.as_ref().unwrap();
    let save = app.core.prepare_file_dialog_request(
        OutputDraft::AudioMix(&session.selection).request(session.session_id),
    );
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
    app.core.settings.remember_source(&path);
    app.core
        .settings
        .remember_output(&directory.join("existing.mkv"));
    let before = app.core.settings.clone();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::BatchInput,
        outcome: DialogOutcome::Cancelled,
    });
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioMixOutput { session_id: 1 },
        outcome: DialogOutcome::Cancelled,
    });

    assert_eq!(app.core.settings, before);
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
        target: DialogTarget::BatchInput,
        outcome: DialogOutcome::Selected(source_path),
    });
    assert_eq!(
        app.core.settings.source_directory(),
        Some(directory.clone())
    );
    assert_eq!(
        app.core.settings.output_directory(Path::new("missing")),
        None
    );

    app.core.discovery = MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
        "missing-ffmpeg",
        "missing-ffprobe",
    )));
    let source_path = directory.join("recording.mkv");
    app.load_media_path_with_probe(&source_path, |path| Ok(media(path)));
    let session_id = app.state.audio_mix.as_ref().unwrap().session_id;
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioMixOutput { session_id },
        outcome: DialogOutcome::Selected(output_directory.join("chosen.mkv")),
    });
    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::AudioMixOutput { session_id: 999 },
        outcome: DialogOutcome::Selected(directory.join("stale.mkv")),
    });
    assert_eq!(
        app.core.settings.source_directory(),
        Some(directory.clone())
    );
    assert_eq!(
        app.core.settings.output_directory(Path::new("missing")),
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
        .request(FileDialogRequest::OpenSources {
            initial_directory: None,
            target: DialogTarget::BatchInput,
        })
        .unwrap();

    assert!(!app.file_intake_enabled());
    assert_workspace(&app.core, Operation::AudioMix);
    assert_eq!(
        app.file_dialog.request(FileDialogRequest::OpenSources {
            initial_directory: None,
            target: DialogTarget::BatchInput,
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
        .picker_error
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
        assert!(app.show_upscale_picker_error(ui));
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
fn batch_append_picker_failure_is_visible_and_never_leaks_to_intake() {
    let (directory, path) = temp_file("batch-append-picker-failure");
    let source = media(&path);
    let operation =
        QueuedOperation::Remux(crate::remux::draft::RemuxDraft::new(&source).request(&source));
    let mut app = app();
    app.core.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::Remux,
        defaults: Some(BatchDefaults::Remux {
            container: MediaContainer::Matroska,
        }),
        items: vec![BatchItem {
            id: BatchItemId(0),
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
    });
    app.core.show_batch_setup();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::BatchAppend,
        outcome: DialogOutcome::Failed("native picker diagnostic".to_owned()),
    });

    assert!(app.core.source.error().is_none());
    let batch = app.core.batch_setup.as_ref().unwrap();
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
fn batch_append_picker_respects_cancellation_and_original_batch_identity() {
    let (directory, path) = temp_file("batch-append-picker-cancel");
    let source = media(&path);
    let operation =
        QueuedOperation::Remux(crate::remux::draft::RemuxDraft::new(&source).request(&source));
    let mut app = app();
    app.core.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::NoSource,
        operation: Operation::Remux,
        defaults: Some(BatchDefaults::Remux {
            container: MediaContainer::Matroska,
        }),
        items: vec![BatchItem {
            id: BatchItemId(0),
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
    });
    app.core.show_batch_setup();
    app.batch_append_error_details_open = true;
    let before = app.core.batch_setup.clone();

    app.settle_file_dialog(FileDialogCompletion {
        target: DialogTarget::BatchAppend,
        outcome: DialogOutcome::Cancelled,
    });

    assert_eq!(app.core.batch_setup, before);
    assert!(app.batch_append_error_details_open);
    assert!(app.core.source.error().is_none());
    assert!(app.core.queue.scheduler().jobs().is_empty());
    let selected_directory = directory.join("additional");
    fs::create_dir_all(&selected_directory).unwrap();
    let selected = selected_directory.join("additional.mkv");
    fs::write(&selected, b"media").unwrap();
    let (wake_tx, wake_rx) = mpsc::channel();
    let path = selected.clone();
    app.file_dialog = FileDialogOwner::new_for_test(
        move || {
            let _ = wake_tx.send(());
        },
        move || Ok(move |_: &FileDialogRequest| Ok(Some(path.clone()))),
    );
    let request = FileDialogRequest::OpenSources {
        initial_directory: None,
        target: DialogTarget::BatchAppend,
    };
    app.start_file_dialog(request.clone());
    assert!(app.finish_batch_setup());
    app.core.batch_setup = before.clone();
    app.core.show_batch_setup();
    wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    app.poll_file_dialog();
    assert_eq!(app.core.batch_setup, before);
    assert!(app.core.settings.source_directory().is_none());

    app.start_file_dialog(request);
    wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    app.poll_file_dialog();
    let batch = app.core.batch_setup.as_ref().unwrap();
    assert_eq!(batch.items.len(), 2);
    assert_eq!(batch.items[0], before.as_ref().unwrap().items[0]);
    assert_eq!(batch.items[1].path, selected);
    assert_eq!(
        app.core.settings.source_directory(),
        Some(selected_directory)
    );
    assert!(app.finish_batch_setup());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn late_input_completion_cannot_change_a_returned_route_or_directory_history() {
    let (directory, path) = temp_file("late-input-dialog");
    let mut app = app();
    let (wake_tx, wake_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let selected = path.clone();
    app.file_dialog = FileDialogOwner::new_for_test(
        move || {
            let _ = wake_tx.send(());
        },
        move || {
            Ok(move |_: &FileDialogRequest| {
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                Ok(Some(selected.clone()))
            })
        },
    );
    app.start_file_dialog(FileDialogRequest::OpenSources {
        initial_directory: None,
        target: DialogTarget::BatchInput,
    });
    assert!(app.file_dialog.is_pending());
    // The UI blocks navigation during a picker; a forced core route change still retires it.
    app.enter_queue();
    assert_eq!(app.core.navigation(), NavigationState::NoSource);
    app.core.enter_queue();
    app.core.return_from_queue();
    release_tx.send(()).unwrap();
    wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    app.poll_file_dialog();
    assert!(!app.file_dialog.is_pending());
    assert!(!app.core.source.is_inspecting());
    assert!(app.core.source.current().is_none());
    assert!(app.core.settings.source_directory().is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn output_dialogs_are_isolated_between_queue_batch_and_normal_drafts() {
    let (directory, path) = temp_file("isolated-editor-dialogs");
    let chosen_directory = directory.join("chosen");
    fs::create_dir(&chosen_directory).unwrap();
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let normal = app
        .state
        .audio_mix
        .as_ref()
        .unwrap()
        .selection
        .output_path();
    let job = app
        .enqueue_current_operation(Operation::AudioMix)
        .unwrap()
        .unwrap();
    assert!(app.begin_queue_edit(job, QueueEditOrigin::SourceWorkspace));
    let queue_request = editor_output_dialog_request(&app.core.queue_edit().unwrap().editor);
    let queue_target = queue_request.target();
    let queue_path = chosen_directory.join("queue.mp4");
    app.settle_file_dialog(FileDialogCompletion {
        target: queue_target,
        outcome: DialogOutcome::Selected(queue_path.clone()),
    });
    assert_eq!(
        app.core.queue_edit().unwrap().audio_mix.output_path(),
        queue_path
    );
    assert_eq!(
        app.state
            .audio_mix
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        normal
    );
    assert!(app.finish_queue_edit());
    let source = media(&path);
    app.core.batch_setup = Some(BatchSetupState {
        origin: BatchOrigin::SourceWorkspace {
            active_operation: Operation::AudioMix,
        },
        operation: Operation::AudioMix,
        defaults: None,
        items: vec![BatchItem {
            id: BatchItemId(0),
            path: path.clone(),
            included: true,
            source: ready_batch_source(source),
            operation: None,
            readiness: BatchReadiness::Ready,
            reason: None,
            overridden: false,
        }],
        error: None,
        append_error: None,
    });
    app.core.show_batch_setup();
    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(0))));
    let batch_request = editor_output_dialog_request(&app.core.batch_edit.as_ref().unwrap().editor);
    let batch_target = batch_request.target();
    let batch_before = app
        .core
        .batch_edit
        .as_ref()
        .unwrap()
        .audio_mix
        .output_path();
    // A late result from the retired queue editor must not affect the batch or directory memory.
    app.settle_file_dialog(FileDialogCompletion {
        target: queue_target,
        outcome: DialogOutcome::Selected(directory.join("stale.mp4")),
    });
    assert_eq!(
        app.core
            .batch_edit
            .as_ref()
            .unwrap()
            .audio_mix
            .output_path(),
        batch_before
    );
    assert_eq!(
        app.core.settings.output_directory(&directory),
        Some(chosen_directory.clone())
    );
    let batch_path = chosen_directory.join("batch.mp4");
    app.settle_file_dialog(FileDialogCompletion {
        target: batch_target,
        outcome: DialogOutcome::Selected(batch_path.clone()),
    });
    assert_eq!(
        app.core
            .batch_edit
            .as_ref()
            .unwrap()
            .audio_mix
            .output_path(),
        batch_path
    );
    assert!(app.finish_batch_edit());
    assert!(app.begin_batch_edit(BatchEditTarget::Item(BatchItemId(0))));
    let before = app
        .core
        .batch_edit
        .as_ref()
        .unwrap()
        .audio_mix
        .output_path();
    app.settle_file_dialog(FileDialogCompletion {
        target: batch_target,
        outcome: DialogOutcome::Failed("late failure".to_owned()),
    });
    assert!(app.core.batch_edit.as_ref().unwrap().error.is_none());
    assert_eq!(
        app.core
            .batch_edit
            .as_ref()
            .unwrap()
            .audio_mix
            .output_path(),
        before
    );
    assert_eq!(
        app.state
            .audio_mix
            .as_ref()
            .unwrap()
            .selection
            .output_path(),
        normal
    );
    fs::remove_dir_all(directory).unwrap();
}

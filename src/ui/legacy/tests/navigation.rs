use super::*;

#[test]
fn initialization_has_no_source_without_starting_an_executor() {
    let app = app();

    assert_no_source(&app.core);
    assert!(app.state.audio_mix.is_none());
    assert!(app.state.upscale.is_none());
    assert!(app.state.remux.is_none());
    assert!(app.state.audio_extract.is_none());
    assert!(app.state.frame_capture.is_none());
    assert!(app.core.executors_idle());
    assert!(app.core.media_queue.jobs().is_empty());
}

#[test]
fn fresh_application_state_renders_unified_file_intake() {
    let mut app = app();
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    let mut output = context.run_ui(Default::default(), |ui| {
        app.show_navigation(ui);
    });

    assert!(output_contains_text(&output, "Open files"));
    assert!(!output_contains_text(&output, "Audio Mix"));
    assert!(!output_contains_text(&output, "Upscale"));
    output.textures_delta.clear();
}

#[test]
fn idle_no_source_surface_schedules_no_application_repaint() {
    let mut app = app();
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    theme::set_motion_durations(&context, theme::MotionDurations::DISABLED);

    let repaint_delay = settled_navigation_repaint_delay(&context, &mut app);

    assert_eq!(repaint_delay, Duration::MAX);
}

#[test]
fn idle_source_workspace_schedules_no_application_repaint() {
    let (directory, path) = temp_file("idle-workspace-repaint");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    theme::set_motion_durations(&context, theme::MotionDurations::DISABLED);

    let repaint_delay = settled_navigation_repaint_delay(&context, &mut app);

    assert_eq!(repaint_delay, Duration::MAX);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn inspecting_state_is_sparse_and_blocks_source_replacement() {
    let (directory, path) = temp_file("inspecting-state");
    let mut app = app();
    let _sender = begin_pending_inspection(&mut app, &path);
    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);

    assert_eq!(app.core.navigation(), NavigationState::NoSource);
    assert!(app.media_inspection.is_some());
    assert!(!app.file_intake_enabled());

    let mut output = context.run_ui(Default::default(), |ui| {
        app.show_navigation(ui);
    });

    assert!(output_contains_text(&output, "Inspecting media…"));
    assert!(!output_contains_text(&output, "Audio Mix"));
    assert!(!output_contains_text(&output, "Open files"));
    assert!(!output_contains_text(&output, "or drop a video anywhere"));
    output.textures_delta.clear();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn operation_selection_is_navigation_inside_a_loaded_workspace() {
    let (directory, path) = temp_file("mode-switch");
    let mut app = app();

    app.select_tool(Operation::VideoUpscale);
    assert_no_source(&app.core);

    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    assert_workspace(&app.core, Operation::AudioMix);
    let source_path = app.core.media_intake.current().unwrap().path.clone();

    for operation in [
        Operation::VideoUpscale,
        Operation::Remux,
        Operation::AudioExtract,
        Operation::FrameCapture,
        Operation::AudioMix,
    ] {
        app.select_tool(operation);
        assert_workspace(&app.core, operation);
        assert_eq!(app.core.media_intake.current().unwrap().path, source_path);
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn close_source_returns_to_true_empty_intake() {
    let (directory, path) = temp_file("close-source");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    for operation in [
        Operation::VideoUpscale,
        Operation::Remux,
        Operation::AudioExtract,
        Operation::FrameCapture,
    ] {
        app.select_tool(operation);
    }

    app.handle_source_action(SourceAction::CloseSource);

    assert_no_source(&app.core);
    assert!(app.state.audio_mix.is_none());
    assert!(app.state.upscale.is_none());
    assert!(app.state.remux.is_none());
    assert!(app.state.audio_extract.is_none());
    assert!(app.state.frame_capture.is_none());
    assert!(app.state.source_presentation.is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn running_and_terminal_operation_states_hide_workspace_chrome() {
    let non_ready_states = [
        AudioMixExecutionState::Running,
        AudioMixExecutionState::Completed {
            output: PathBuf::from("finished.mp4"),
        },
        AudioMixExecutionState::Failed(MediaExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: "failed".to_owned(),
        }),
        AudioMixExecutionState::Cancelled,
    ];

    assert!(operation_workspace_visible(
        &AudioMixExecutionState::Idle,
        true
    ));
    assert!(!operation_workspace_visible(
        &AudioMixExecutionState::Idle,
        false
    ));
    for state in non_ready_states {
        assert!(!operation_workspace_visible(&state, true));
    }
}

#[test]
fn intake_queue_navigation_and_back_preserve_jobs() {
    let (directory, path) = temp_file("queue-navigation");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.enqueue_from_ready_surface(Operation::AudioMix);
    let job_id = app.core.media_queue.jobs()[0].id();
    app.close_source();

    app.handle_file_intake_action(FileIntakeAction::OpenQueue);
    assert_eq!(
        app.core.navigation(),
        NavigationState::Queue {
            origin: QueueOrigin::NoSource
        }
    );
    assert!(app.handle_queue_action(QueueAction::Back));

    assert_eq!(app.core.navigation(), NavigationState::NoSource);
    assert_eq!(app.core.media_queue.jobs()[0].id(), job_id);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_queue_navigation_restores_the_same_source_and_operation() {
    let (directory, path) = temp_file("workspace-queue-navigation");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let session_id = app.state.remux.as_ref().unwrap().session_id;

    app.enter_queue();
    assert_eq!(
        app.core.navigation(),
        NavigationState::Queue {
            origin: QueueOrigin::SourceWorkspace {
                active_operation: Operation::Remux
            }
        }
    );
    assert!(app.handle_queue_action(QueueAction::Back));

    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(app.core.media_intake.current().unwrap().path, path);
    assert_eq!(app.state.remux.as_ref().unwrap().session_id, session_id);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_back_clears_disposable_history_and_retains_failures() {
    let (directory, path) = temp_file("queue-back-history");
    let mut app = app();
    populate_cancelled_and_failed_queue_history(&mut app, &path, &directory);
    app.enter_queue();

    assert!(app.handle_queue_action(QueueAction::Back));

    assert_eq!(app.core.media_queue.jobs().len(), 1);
    assert!(matches!(
        app.core.media_queue.jobs()[0].state(),
        QueueJobState::Failed(_)
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_add_files_clears_disposable_history_before_opening_intake() {
    let (directory, path) = temp_file("queue-add-files-history");
    let mut app = app();
    populate_cancelled_and_failed_queue_history(&mut app, &path, &directory);
    app.enter_queue();

    assert!(app.handle_queue_action(QueueAction::AddFiles));

    assert_eq!(app.core.media_queue.jobs().len(), 1);
    assert!(matches!(
        app.core.media_queue.jobs()[0].state(),
        QueueJobState::Failed(_)
    ));
    assert!(app.handle_queue_action(QueueAction::ClearFinished));
    assert!(app.core.media_queue.jobs().is_empty());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_edit_hydrates_the_queued_request_and_cancel_preserves_it() {
    let (directory, path) = temp_file("queue-edit-cancel");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let custom_output = directory.join("queued-custom.mp4");
    let selection = &mut app.state.remux.as_mut().unwrap().selection;
    selection.set_container(MediaContainer::Mp4);
    selection.set_custom_output_path(custom_output);
    let job_id = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    let original = app.core.media_queue.jobs()[0].operation().clone();
    app.enter_queue();

    assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
    let edit = app.queue_edit.as_mut().unwrap();
    assert_eq!(edit.active_operation, Operation::Remux);
    assert_eq!(
        QueuedOperation::Remux(edit.remux.request(&edit.media)),
        original
    );
    edit.remux.set_container(MediaContainer::Matroska);

    assert!(app.finish_queue_edit());
    assert_eq!(app.core.media_queue.jobs()[0].id(), job_id);
    assert_eq!(app.core.media_queue.jobs()[0].operation(), &original);
    assert!(matches!(
        app.core.navigation(),
        NavigationState::Queue { .. }
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_queue_edit_targets_exact_job_and_restores_retained_source_without_probe() {
    let (first_directory, first_path) = temp_file("workspace-edit-first");
    let (second_directory, second_path) = temp_file("workspace-edit-second");
    let mut app = app();
    app.load_media_path_with_probe(&first_path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let first_output = first_directory.join("first-output.mp4");
    let first_selection = &mut app.state.remux.as_mut().unwrap().selection;
    first_selection.set_container(MediaContainer::Mp4);
    first_selection.set_custom_output_path(first_output);
    let first_job = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    let first_request = app.core.media_queue.jobs()[0].operation().clone();

    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(first_directory.join("second-output.mkv"));
    let second_job = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    app.load_media_path_with_probe(&second_path, |path| Ok(media(path)));
    assert_eq!(app.core.media_intake.current().unwrap().path, second_path);

    assert!(app.begin_queue_edit(first_job, QueueEditOrigin::SourceWorkspace));
    assert_eq!(app.queue_edit.as_ref().unwrap().job_id, first_job);
    assert_eq!(
        QueuedOperation::Remux(
            app.queue_edit
                .as_ref()
                .unwrap()
                .remux
                .request(&app.queue_edit.as_ref().unwrap().media)
        ),
        first_request
    );
    assert!(app.media_inspection.is_none());

    assert!(app.finish_queue_edit());
    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(app.core.media_intake.current().unwrap().path, first_path);
    assert_eq!(
        QueuedOperation::Remux(
            app.state
                .remux
                .as_ref()
                .unwrap()
                .selection
                .request(&app.state.remux.as_ref().unwrap().media)
        ),
        first_request
    );
    assert!(app.media_inspection.is_none());
    assert_eq!(app.core.media_queue.jobs()[1].id(), second_job);

    fs::remove_dir_all(first_directory).unwrap();
    fs::remove_dir_all(second_directory).unwrap();
}

#[test]
fn workspace_queue_edit_save_returns_to_saved_operation_with_same_job_id() {
    let (directory, path) = temp_file("workspace-edit-save");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let job_id = app
        .enqueue_current_operation(Operation::AudioMix)
        .unwrap()
        .unwrap();

    assert!(app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
    app.queue_edit.as_mut().unwrap().active_operation = Operation::FrameCapture;
    assert!(app.save_queue_edit());

    assert_workspace(&app.core, Operation::FrameCapture);
    assert_eq!(app.core.media_queue.jobs().len(), 1);
    assert_eq!(app.core.media_queue.jobs()[0].id(), job_id);
    assert!(matches!(
        app.core.media_queue.jobs()[0].operation(),
        QueuedOperation::FrameCapture(_)
    ));
    assert!(app.media_inspection.is_none());

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_queue_edit_is_blocked_during_automatic_advancement() {
    let (directory, path) = temp_file("workspace-edit-running");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let job_id = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    // The test harness rejects FFmpeg arguments without touching media. The queue
    // stays active until its owner polls the terminal process event.
    app.media_tools = MediaToolsState::Ready(MediaTools::new(
        std::env::current_exe().unwrap(),
        "missing-ffprobe",
    ));

    assert!(app.start_media_queue());
    assert_ne!(app.core.media_queue.run_state(), QueueRunState::Stopped);
    assert!(!app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
    assert!(app.queue_edit.is_none());
    assert_workspace(&app.core, Operation::Remux);
    assert!(app.core.media_queue.cancel_queue());

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_edit_can_change_operation_with_atomic_same_id_replacement() {
    let (directory, path) = temp_file("queue-edit-replace");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    let job_id = app
        .enqueue_current_operation(Operation::AudioMix)
        .unwrap()
        .unwrap();
    app.enter_queue();
    assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
    app.queue_edit.as_mut().unwrap().active_operation = Operation::FrameCapture;

    assert!(app.save_queue_edit());

    assert_eq!(app.core.media_queue.jobs().len(), 1);
    assert_eq!(app.core.media_queue.jobs()[0].id(), job_id);
    assert!(matches!(
        app.core.media_queue.jobs()[0].operation(),
        QueuedOperation::FrameCapture(_)
    ));
    assert!(matches!(
        app.core.navigation(),
        NavigationState::Queue { .. }
    ));
    assert!(app.queue_edit.is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn app_shutdown_cancels_waiting_queue_work() {
    let (directory, path) = temp_file("queue-shutdown");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    app.enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();

    app.shutdown();

    assert!(matches!(
        app.core.media_queue.jobs()[0].state(),
        QueueJobState::Cancelled
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn switching_tools_reuses_successful_probe_without_probing_again() {
    let (directory, path) = temp_file("reuse");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));

    app.select_tool(Operation::VideoUpscale);

    assert_workspace(&app.core, Operation::VideoUpscale);
    assert_eq!(
        app.state.upscale.as_ref().map(|s| &s.media.path),
        Some(&path)
    );
    assert_eq!(
        app.core.media_intake.current().map(|m| &m.path),
        Some(&path)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn switching_operations_does_not_revalidate_or_clear_the_source() {
    let (directory, path) = temp_file("changed-switch");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    fs::write(&path, b"changed-size").unwrap();

    app.select_tool(Operation::VideoUpscale);

    assert_workspace(&app.core, Operation::VideoUpscale);
    assert!(app.state.upscale.is_some());
    assert_eq!(
        app.core.media_intake.current().map(|media| &media.path),
        Some(&path)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn file_drop_policy_disables_replacement_while_any_operation_runs() {
    assert!(file_intake_enabled(
        &AudioMixExecutionState::Idle,
        &UpscaleExecutionState::Idle,
        &RemuxExecutionState::Idle,
        &AudioExtractExecutionState::Idle,
        &FrameCaptureExecutionState::Idle,
    ));
    assert!(!file_intake_enabled(
        &AudioMixExecutionState::Running,
        &UpscaleExecutionState::Idle,
        &RemuxExecutionState::Idle,
        &AudioExtractExecutionState::Idle,
        &FrameCaptureExecutionState::Idle,
    ));
    assert!(!file_intake_enabled(
        &AudioMixExecutionState::Idle,
        &UpscaleExecutionState::Running,
        &RemuxExecutionState::Idle,
        &AudioExtractExecutionState::Idle,
        &FrameCaptureExecutionState::Idle,
    ));
    assert!(!file_intake_enabled(
        &AudioMixExecutionState::Idle,
        &UpscaleExecutionState::Idle,
        &RemuxExecutionState::Running,
        &AudioExtractExecutionState::Idle,
        &FrameCaptureExecutionState::Idle,
    ));
    assert!(!file_intake_enabled(
        &AudioMixExecutionState::Idle,
        &UpscaleExecutionState::Idle,
        &RemuxExecutionState::Idle,
        &AudioExtractExecutionState::Running,
        &FrameCaptureExecutionState::Idle,
    ));
    assert!(!file_intake_enabled(
        &AudioMixExecutionState::Idle,
        &UpscaleExecutionState::Idle,
        &RemuxExecutionState::Idle,
        &AudioExtractExecutionState::Idle,
        &FrameCaptureExecutionState::Running,
    ));
}

#[test]
fn drag_hover_accepts_single_and_multiple_files_when_intake_is_available() {
    let (first_directory, first) = temp_file("hover-first");
    let (second_directory, second) = temp_file("hover-second");
    let mut app = app();

    assert!(drag_hovered(&app, &[&first]));
    assert!(drag_hovered(&app, &[&first, &second]));

    let _sender = begin_pending_inspection(&mut app, &first);
    assert!(!drag_hovered(&app, &[&second]));

    fs::remove_dir_all(first_directory).unwrap();
    fs::remove_dir_all(second_directory).unwrap();
}

#[test]
fn audio_mix_terminal_states_reject_source_replacement() {
    for state in terminal_execution_states() {
        assert!(!file_intake_enabled(
            &state,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
    }
}

#[test]
fn upscale_terminal_states_reject_source_replacement() {
    for state in terminal_execution_states() {
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &state,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
    }
}

#[test]
fn terminal_reset_to_idle_reenables_file_intake() {
    let completed = AudioMixExecutionState::Completed {
        output: PathBuf::from("finished.mp4"),
    };
    assert!(!file_intake_enabled(
        &completed,
        &UpscaleExecutionState::Idle,
        &RemuxExecutionState::Idle,
        &AudioExtractExecutionState::Idle,
        &FrameCaptureExecutionState::Idle,
    ));

    assert!(file_intake_enabled(
        &AudioMixExecutionState::Idle,
        &UpscaleExecutionState::Idle,
        &RemuxExecutionState::Idle,
        &AudioExtractExecutionState::Idle,
        &FrameCaptureExecutionState::Idle,
    ));
}

#[test]
fn root_update_defers_source_completion_until_tools_are_ready() {
    let (directory, path) = temp_file("root-readiness");
    let mut app = app();
    let inspection = begin_pending_inspection(&mut app, &path);
    inspection.send(Ok(media(&path))).unwrap();

    let context = egui::Context::default();
    fonts::install(&context);
    theme::install(&context);
    let render = |app: &mut ApolloApp| {
        context
            .run_ui(Default::default(), |ui| app.show_root(ui))
            .drop_without_applying_deltas();
    };

    app.media_tools = MediaToolsState::Checking { attempt_id: 1 };
    render(&mut app);
    assert_no_source(&app.core);
    assert!(app.media_inspection.is_some());

    app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
    render(&mut app);
    assert_no_source(&app.core);
    assert!(app.media_inspection.is_some());

    app.media_tools = MediaToolsState::Ready(MediaTools::new("missing-ffmpeg", "missing-ffprobe"));
    render(&mut app);
    assert_workspace(&app.core, Operation::AudioMix);
    assert_eq!(app.core.media_intake.current().unwrap().path, path);
    assert!(app.media_inspection.is_none());
    fs::remove_dir_all(directory).unwrap();
}

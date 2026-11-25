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
    assert!(app.file_intake_enabled());
    assert!(app.core.queue.scheduler().jobs().is_empty());
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
    assert!(app.core.source.is_inspecting());
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
    let source_path = app.core.source.current().unwrap().path.clone();

    for operation in [
        Operation::VideoUpscale,
        Operation::Remux,
        Operation::AudioExtract,
        Operation::FrameCapture,
        Operation::AudioMix,
    ] {
        app.select_tool(operation);
        assert_workspace(&app.core, operation);
        assert_eq!(app.core.source.current().unwrap().path, source_path);
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
fn intake_queue_navigation_and_back_preserve_jobs() {
    let (directory, path) = temp_file("queue-navigation");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.handle_audio_mix_action(AudioMixSelectionAction::QueueRequested);
    let job_id = app.core.queue.scheduler().jobs()[0].id();
    let replacement = directory.join("replacement.mkv");
    fs::write(&replacement, b"replacement").unwrap();
    let sender = begin_pending_inspection(&mut app, &replacement);
    app.close_source();
    assert!(sender.send(Ok(media(&replacement))).is_err());
    app.poll_media_inspection();
    assert_no_source(&app.core);
    assert!(app.state.source_presentation.is_none());

    app.handle_file_intake_action(FileIntakeAction::OpenQueue);
    assert_eq!(
        app.core.navigation(),
        NavigationState::Queue {
            origin: QueueOrigin::NoSource
        }
    );
    assert!(app.handle_queue_action(QueueAction::Back));

    assert_eq!(app.core.navigation(), NavigationState::NoSource);
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), job_id);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_queue_navigation_restores_the_same_source_and_operation() {
    let (directory, path) = temp_file("workspace-queue-navigation");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let draft = app.state.remux.as_mut().unwrap();
    draft
        .selection
        .set_custom_output_path(directory.join("custom.mkv"));
    let session_id = draft.session_id;
    let selection = draft.selection.clone();

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
    assert_eq!(app.core.source.current().unwrap().path, path);
    assert_eq!(app.state.remux.as_ref().unwrap().session_id, session_id);
    assert_eq!(app.state.remux.as_ref().unwrap().selection, selection);
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
    let first_request = app.core.queue.scheduler().jobs()[0].operation().clone();

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
    assert_eq!(app.core.source.current().unwrap().path, second_path);

    assert!(app.begin_queue_edit(first_job, QueueEditOrigin::SourceWorkspace));
    assert_eq!(app.core.queue_edit().unwrap().job_id, first_job);
    assert_eq!(
        QueuedOperation::Remux(
            app.core
                .queue_edit()
                .unwrap()
                .remux
                .request(&app.core.queue_edit().unwrap().media)
        ),
        first_request
    );
    assert!(!app.core.source.is_inspecting());

    assert!(app.finish_queue_edit());
    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(app.core.source.current().unwrap().path, first_path);
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
    assert!(!app.core.source.is_inspecting());
    assert_eq!(app.core.queue.scheduler().jobs()[1].id(), second_job);

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
    app.core.queue_edit_mut().unwrap().active_operation = Operation::FrameCapture;
    assert!(app.save_queue_edit());

    assert_workspace(&app.core, Operation::FrameCapture);
    assert_eq!(app.core.queue.scheduler().jobs().len(), 1);
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), job_id);
    assert!(matches!(
        app.core.queue.scheduler().jobs()[0].operation(),
        QueuedOperation::FrameCapture(_)
    ));
    assert!(!app.core.source.is_inspecting());

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_queue_edit_is_blocked_during_automatic_advancement() {
    use crate::app::queue::QueueEffect;
    use crate::media::process::{MediaProcessEvent, MediaProcessWake};

    let (directory, path) = temp_file("workspace-edit-running");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let job_id = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(directory.join("second.mkv"));
    let waiting_id = app
        .enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    // The test harness rejects FFmpeg arguments without touching media. The queue
    // stays active until its owner polls the terminal process event.
    app.core.discovery = MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
        std::env::current_exe().unwrap(),
        "missing-ffprobe",
    )));

    let (sender, receiver) = mpsc::channel();
    assert!(matches!(
        app.core.queue_action(QueueAction::Start, || {
            let sender = sender.clone();
            MediaProcessWake::new(move |event| {
                if event == MediaProcessEvent::Terminal {
                    let _ = sender.send(());
                }
            })
        }),
        QueueEffect::Handled(true)
    ));
    assert_ne!(
        app.core.queue.scheduler().run_state(),
        QueueRunState::Stopped
    );
    app.enter_queue();
    assert_eq!(app.core.queue.scheduler().active_job(), Some(job_id));
    assert_eq!(app.core.queue.scheduler().pending_count(), 2);
    assert!(app.handle_queue_action(QueueAction::Back));
    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(app.core.queue.scheduler().active_job(), Some(job_id));
    assert!(!app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
    assert!(app.core.queue_edit().is_none());
    app.handle_remux_action(RemuxSelectionAction::RemuxRequested);
    assert_eq!(app.core.queue.scheduler().pending_count(), 2);
    assert_workspace(&app.core, Operation::Remux);
    assert!(app.handle_queue_action(QueueAction::StopAfterCurrent));
    assert_eq!(
        app.core.queue.scheduler().run_state(),
        QueueRunState::StoppingAfterCurrent
    );
    app.core.discovery =
        MediaToolsDiscovery::with_state(MediaToolsState::Unavailable(MediaToolsError::MissingBoth));
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    app.core
        .poll_media_queue(|| panic!("stopped queue must not advance"));
    let queue = app.core.queue.scheduler();
    assert_eq!(queue.run_state(), QueueRunState::Stopped);
    assert!(queue.active_job().is_none());
    assert_eq!(queue.jobs()[0].id(), job_id);
    assert!(queue.jobs()[0].state().is_terminal());
    assert_eq!(queue.jobs()[1].id(), waiting_id);
    assert_eq!(queue.jobs()[1].state(), &QueueJobState::Waiting);
    assert!(app.handle_queue_action(QueueAction::CancelQueue));
    assert_eq!(
        app.core.queue.scheduler().jobs()[1].state(),
        &QueueJobState::Cancelled
    );

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
    assert_eq!(app.core.source.current().map(|m| &m.path), Some(&path));
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
        app.core.source.current().map(|media| &media.path),
        Some(&path)
    );
    fs::remove_dir_all(directory).unwrap();
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

    app.core.discovery = MediaToolsDiscovery::with_state(MediaToolsState::Checking);
    render(&mut app);
    assert_no_source(&app.core);
    assert!(app.core.source.is_inspecting());

    app.core.discovery =
        MediaToolsDiscovery::with_state(MediaToolsState::Unavailable(MediaToolsError::MissingBoth));
    render(&mut app);
    assert_no_source(&app.core);
    assert!(app.core.source.is_inspecting());

    app.core.discovery = MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
        "missing-ffmpeg",
        "missing-ffprobe",
    )));
    render(&mut app);
    assert_workspace(&app.core, Operation::AudioMix);
    assert_eq!(app.core.source.current().unwrap().path, path);
    assert!(!app.core.source.is_inspecting());
    fs::remove_dir_all(directory).unwrap();
}

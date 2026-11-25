use super::*;

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

    for label in [
        "Edit",
        "Remove",
        "↑",
        "↓",
        "Start queue",
        "Cancel queue",
        "Back",
    ] {
        assert!(output_contains_text(&output, label), "missing {label:?}");
    }
    output.textures_delta.clear();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_cancel_requires_confirmation_and_dismissal_keeps_waiting_jobs() {
    let (directory, path) = temp_file("queue-cancel-confirmation");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    app.enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
    app.enter_queue();

    app.handle_queue_view_action(QueueAction::CancelQueue);
    assert!(app.confirm_cancel_queue);
    assert_eq!(
        app.core.queue.scheduler().jobs()[0].state(),
        &QueueJobState::Waiting
    );
    app.resolve_queue_cancellation(false);
    assert!(!app.confirm_cancel_queue);
    assert_eq!(
        app.core.queue.scheduler().jobs()[0].state(),
        &QueueJobState::Waiting
    );

    app.handle_queue_view_action(QueueAction::CancelQueue);
    app.resolve_queue_cancellation(true);
    assert!(!app.confirm_cancel_queue);
    assert_eq!(
        app.core.queue.scheduler().jobs()[0].state(),
        &QueueJobState::Cancelled
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_edit_accepts_shared_destination_and_keeps_job_identity() {
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
    let duplicate_output = app.core.queue.scheduler().jobs()[1]
        .operation()
        .output()
        .to_path_buf();
    app.enter_queue();
    assert!(app.handle_queue_action(QueueAction::Edit(first)));
    app.core
        .queue_edit_mut()
        .unwrap()
        .remux
        .set_custom_output_path(duplicate_output.clone());
    assert!(app.save_queue_edit());
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), first);
    assert_eq!(
        app.core.queue.scheduler().jobs()[0].operation().output(),
        duplicate_output
    );
    assert_eq!(app.core.queue.scheduler().jobs()[1].id(), second);
    assert!(app.core.queue_edit().is_none());
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
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), second);
    assert_eq!(app.core.queue.scheduler().jobs()[1].id(), first);
    assert!(app.handle_queue_action(QueueAction::MoveLater(second)));
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), first);
    assert_eq!(app.core.queue.scheduler().jobs()[1].id(), second);
    fs::remove_dir_all(directory).unwrap();
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
    assert!(app.core.queue.scheduler().jobs().is_empty());

    let terminal = app
        .enqueue_current_operation(Operation::AudioMix)
        .unwrap()
        .unwrap();
    assert!(app.handle_queue_action(QueueAction::CancelQueue));
    assert!(!app.handle_queue_action(QueueAction::Remove(terminal)));
    assert!(app.handle_queue_action(QueueAction::ClearFinished));
    assert!(app.core.queue.scheduler().jobs().is_empty());
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

    assert!(app.handle_queue_action(QueueAction::Start));

    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(app.state.remux.as_ref().unwrap().session_id, session_id);
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), job_id);
    assert!(matches!(
        app.core.queue.scheduler().jobs()[0].state(),
        QueueJobState::Failed(QueueJobFailure::ProcessStart(_))
    ));
    assert!(app.core.execution_arbitration().queue_start_allowed());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_back_retains_terminal_history_across_source_changes() {
    let (directory, path) = temp_file("queue-back-history");
    let mut app = app();
    populate_cancelled_and_failed_queue_history(&mut app, &path, &directory);
    app.enter_queue();

    assert!(app.handle_queue_action(QueueAction::Back));
    app.select_tool(Operation::AudioMix);
    app.close_source();
    let replacement = directory.join("replacement.mkv");
    fs::write(&replacement, b"replacement").unwrap();
    app.load_media_path_with_probe(&replacement, |path| Ok(media(path)));
    app.enter_queue();

    assert_eq!(app.core.queue.scheduler().jobs().len(), 2);
    assert!(matches!(
        app.core.queue.scheduler().jobs()[0].state(),
        QueueJobState::Cancelled
    ));
    assert!(matches!(
        app.core.queue.scheduler().jobs()[1].state(),
        QueueJobState::Failed(_)
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queue_add_files_preserves_terminal_history_until_explicit_clear() {
    let (directory, path) = temp_file("queue-add-files-history");
    let mut app = app();
    populate_cancelled_and_failed_queue_history(&mut app, &path, &directory);
    app.enter_queue();

    let (wake_tx, wake_rx) = mpsc::channel();
    app.file_dialog = FileDialogOwner::new_for_test(
        move || {
            let _ = wake_tx.send(());
        },
        || Ok(|_: &FileDialogRequest| Ok(None)),
    );
    let origin = app.core.navigation();
    assert!(app.handle_queue_action(QueueAction::AddFiles));
    wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    app.poll_file_dialog();
    assert!(!app.file_dialog.is_pending());
    assert_eq!(app.core.navigation(), origin);

    assert_eq!(app.core.queue.scheduler().jobs().len(), 2);
    assert!(matches!(
        app.core.queue.scheduler().jobs()[0].state(),
        QueueJobState::Cancelled
    ));
    assert!(matches!(
        app.core.queue.scheduler().jobs()[1].state(),
        QueueJobState::Failed(_)
    ));
    assert!(app.handle_queue_action(QueueAction::ClearFinished));
    assert!(app.core.queue.scheduler().jobs().is_empty());
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
    let original = app.core.queue.scheduler().jobs()[0].operation().clone();
    app.enter_queue();

    assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
    let edit = app.core.queue_edit_mut().unwrap();
    assert_eq!(edit.active_operation, Operation::Remux);
    assert_eq!(
        QueuedOperation::Remux(edit.remux.request(&edit.media)),
        original
    );
    edit.remux.set_container(MediaContainer::Matroska);

    assert!(app.finish_queue_edit());
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), job_id);
    assert_eq!(app.core.queue.scheduler().jobs()[0].operation(), &original);
    assert!(matches!(
        app.core.navigation(),
        NavigationState::Queue { .. }
    ));
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
    app.core.queue_edit_mut().unwrap().active_operation = Operation::FrameCapture;

    assert!(app.save_queue_edit());

    assert_eq!(app.core.queue.scheduler().jobs().len(), 1);
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), job_id);
    assert!(matches!(
        app.core.queue.scheduler().jobs()[0].operation(),
        QueuedOperation::FrameCapture(_)
    ));
    assert!(matches!(
        app.core.navigation(),
        NavigationState::Queue { .. }
    ));
    assert!(app.core.queue_edit().is_none());
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
        app.core.queue.scheduler().jobs()[0].state(),
        QueueJobState::Cancelled
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn enqueue_rejects_stale_source_and_preserves_workspace_until_reopened() {
    let (directory, path) = temp_file("stale-queue-admission");
    let mut app = app();
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    let output = directory.join("custom.mkv");
    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(output.clone());
    let selection = app.state.remux.as_ref().unwrap().selection.clone();
    fs::write(
        &path,
        b"source changed after inspection with different length",
    )
    .unwrap();
    app.enqueue_from_ready_surface(Operation::Remux);
    assert!(app.core.queue.scheduler().jobs().is_empty());
    assert!(app.core.queue_confirmation.is_none());
    assert_eq!(
        app.core.operation_action_error.as_ref().unwrap().1.kind(),
        MediaErrorKind::SourceChanged
    );
    assert_workspace(&app.core, Operation::Remux);
    assert_eq!(app.state.remux.as_ref().unwrap().selection, selection);
    app.load_media_path_with_probe(&path, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    app.enqueue_from_ready_surface(Operation::Remux);
    assert_eq!(app.core.queue.scheduler().jobs().len(), 1);
    assert_eq!(app.core.queue_confirmation, Some(Operation::Remux));
    assert!(app.core.operation_action_error.is_none());
    assert_eq!(
        app.core.queue.scheduler().jobs()[0].source_identity(),
        app.core.source.current_with_identity().unwrap().0
    );
    fs::remove_dir_all(directory).unwrap();
}

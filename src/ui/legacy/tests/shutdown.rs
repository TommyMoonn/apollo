use super::*;
use crate::ui::legacy::shutdown::ExitPresentation;
use egui::{RawInput, ViewportCommand, ViewportEvent, ViewportId};

// Exercise the same logic-only path that eframe uses for an occluded window.
fn close_tick(app: &mut ApolloApp, requested: bool) -> Vec<ViewportCommand> {
    let context = app.egui_context.clone();
    let mut input = RawInput::default();
    if requested {
        input
            .viewports
            .get_mut(&ViewportId::ROOT)
            .unwrap()
            .events
            .push(ViewportEvent::Close);
    }
    context
        .run_logic(&input, |ctx| app.handle_close_request(ctx))
        .viewport_commands
        .remove(&ViewportId::ROOT)
        .unwrap_or_default()
}

fn prepare_remux(app: &mut ApolloApp, source: &Path, output: &Path) {
    app.load_media_path_with_probe(source, |path| Ok(media(path)));
    app.select_tool(Operation::Remux);
    app.state
        .remux
        .as_mut()
        .unwrap()
        .selection
        .set_custom_output_path(output.to_owned());
    app.enqueue_current_operation(Operation::Remux)
        .unwrap()
        .unwrap();
}

#[test]
fn empty_and_terminal_only_sessions_close_without_confirmation() {
    for terminal in [false, true] {
        let (directory, source) = temp_file("close-idle");
        let mut app = app();
        let output = directory.join("existing.mkv");
        fs::write(&output, b"existing").unwrap();
        if terminal {
            prepare_remux(&mut app, &source, &output);
            assert!(app.core.queue.cancel());
        }
        let commands = close_tick(&mut app, true);
        assert!(commands.contains(&ViewportCommand::Close));
        assert_eq!(app.exit, ExitPresentation::Authorized);
        assert_eq!(
            app.core.queue.scheduler().jobs().len(),
            usize::from(terminal)
        );
        assert!(close_tick(&mut app, true).is_empty());
        drop(app);
        assert_eq!(fs::read(output).unwrap(), b"existing");
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn declining_close_preserves_waiting_jobs_and_draft_then_accepting_blocks_new_work() {
    let (directory, source) = temp_file("close-waiting");
    let output = directory.join("existing.mkv");
    fs::write(&output, b"original").unwrap();
    let mut app = app();
    prepare_remux(&mut app, &source, &output);
    let draft = app.state.remux.as_ref().unwrap().selection.clone();
    let route = app.core.navigation();
    let id = app.core.queue.scheduler().jobs()[0].id();
    for _ in 0..2 {
        let commands = close_tick(&mut app, true);
        assert!(commands.contains(&ViewportCommand::CancelClose));
        assert!(!commands.contains(&ViewportCommand::Close));
        assert_eq!(app.exit, ExitPresentation::Confirm);
        assert!(!app.core.is_closing());
    }
    app.resolve_exit_confirmation(false);
    assert_eq!(app.exit, ExitPresentation::Open);
    assert_eq!(app.core.navigation(), route);
    assert_eq!(app.state.remux.as_ref().unwrap().selection, draft);
    assert_eq!(app.core.queue.scheduler().jobs()[0].id(), id);
    assert_eq!(
        app.core.queue.scheduler().jobs()[0].state(),
        &QueueJobState::Waiting
    );
    assert!(app.core.queue_start_allowed());
    close_tick(&mut app, true);
    app.resolve_exit_confirmation(true);
    assert!(app.core.is_closing());
    assert!(!app.core.begin_shutdown());
    assert!(!app.core.queue_start_allowed());
    assert!(!app.core.clipboard_start_allowed());
    assert!(!app.file_intake_enabled());
    assert!(!app.handle_queue_action(QueueAction::Start));
    assert!(!app.handle_queue_action(QueueAction::Retry(id)));
    assert_eq!(
        app.enqueue_current_operation(Operation::Remux).unwrap(),
        None
    );
    assert_eq!(
        app.core.queue.scheduler().jobs()[0].state(),
        &QueueJobState::Cancelled
    );
    assert!(close_tick(&mut app, true).contains(&ViewportCommand::Close));
    drop(app);
    let fresh = super::app();
    assert!(fresh.core.queue.scheduler().jobs().is_empty());
    assert_eq!(fs::read(output).unwrap(), b"original");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn closing_waits_for_native_dialog_without_applying_its_result() {
    let (directory, source) = temp_file("close-dialog");
    let mut app = app();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    app.file_dialog = FileDialogOwner::new_for_test(
        move || {
            let _ = wake_tx.send(());
        },
        move || {
            Ok(move |_: &FileDialogRequest| {
                started_tx.send(()).unwrap();
                let source = release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                Ok(Some(source))
            })
        },
    );
    app.start_file_dialog(FileDialogRequest::OpenSources {
        initial_directory: None,
        target: DialogTarget::BatchInput,
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let commands = close_tick(&mut app, true);
    assert!(commands.contains(&ViewportCommand::CancelClose));
    assert!(!commands.contains(&ViewportCommand::Close));
    assert_eq!(app.exit, ExitPresentation::Closing);
    release_tx.send(source).unwrap();
    wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(close_tick(&mut app, false).contains(&ViewportCommand::Close));
    assert!(app.core.source.current().is_none());
    assert!(app.core.batch_setup.is_none());
    fs::remove_dir_all(directory).unwrap();
}

// The scheduler and actual process owner are used here. A shell fixture supplies a
// deterministic progress/terminal notification; no external FFmpeg or timing sleeps
// are required for the assertions. Windows process tests cover its native child API.
#[cfg(unix)]
#[test]
fn confirmed_close_settles_active_output_and_never_launches_the_next_job() {
    use crate::app::queue::QueueEffect;
    use crate::media::process::{MediaProcessEvent, MediaProcessWake};
    use std::os::unix::fs::PermissionsExt;

    for (stop_after_current, already_cancelling, cleanup_failure, completed) in [
        (false, false, false, false),
        (true, false, false, false),
        (false, true, false, false),
        (false, false, true, false),
        (false, false, false, true),
    ] {
        let (directory, source) = temp_file("close-active");
        let executable = directory.join("ffmpeg-fixture");
        let output = directory.join("existing.mkv");
        fs::write(&output, b"original").unwrap();
        let produce = if cleanup_failure {
            "mkdir -- \"$out\""
        } else {
            "printf partial > \"$out\""
        };
        let terminate = if completed { "exit 0" } else { "exec sleep 60" };
        fs::write(&executable, format!("#!/bin/sh\nfor out do :; done\n{produce}\nprintf 'progress=continue\\n'\n{terminate}\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mut app = super::app();
        app.core.discovery = MediaToolsDiscovery::with_state(MediaToolsState::Ready(
            MediaTools::new(&executable, "missing-ffprobe"),
        ));
        prepare_remux(&mut app, &source, &output);
        let next = directory.join("next.mkv");
        app.state
            .remux
            .as_mut()
            .unwrap()
            .selection
            .set_custom_output_path(next.clone());
        app.enqueue_current_operation(Operation::Remux)
            .unwrap()
            .unwrap();
        app.enter_queue();
        let (tx, rx) = mpsc::channel();
        assert!(matches!(
            app.core.queue_action(QueueAction::Start, || {
                let tx = tx.clone();
                MediaProcessWake::new(move |event| {
                    let _ = tx.send(event);
                })
            }),
            QueueEffect::Handled(true)
        ));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            MediaProcessEvent::Progress
        );
        let staging = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_dir())
            .unwrap();
        let active = app.core.queue.scheduler().active_job().unwrap();
        if stop_after_current {
            assert!(app.handle_queue_action(QueueAction::StopAfterCurrent));
        }
        if already_cancelling {
            assert!(app.handle_queue_action(QueueAction::CancelCurrent));
        }
        if completed {
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(10)).unwrap(),
                MediaProcessEvent::Terminal
            );
        }
        // Confirm before consuming the active job's terminal notification. This also
        // covers completion racing cancellation; the successful output must survive.
        app.request_close();
        assert_eq!(app.exit, ExitPresentation::Confirm);
        app.resolve_exit_confirmation(true);
        assert_eq!(app.exit, ExitPresentation::Closing);
        assert_eq!(app.core.queue.scheduler().active_job(), Some(active));
        assert!(staging.exists());
        assert_eq!(
            app.core.queue.scheduler().jobs()[1].state(),
            &QueueJobState::Cancelled
        );
        assert!(!app.handle_queue_action(QueueAction::Start));
        if !completed {
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(10)).unwrap(),
                MediaProcessEvent::Terminal
            );
        }
        let commands = close_tick(&mut app, true);
        assert_eq!(app.core.queue.scheduler().active_job(), None);
        assert_eq!(
            app.core.queue.scheduler().run_state(),
            QueueRunState::Stopped
        );
        if cleanup_failure {
            assert!(matches!(
                app.core.queue.scheduler().jobs()[0].state(),
                QueueJobState::Failed(QueueJobFailure::Execution(
                    crate::media::execution::MediaExecutionFailure::Output { .. }
                ))
            ));
            assert!(app.core.shutdown_failure().is_some());
            assert!(staging.exists());
            assert!(!commands.contains(&ViewportCommand::Close));
            assert!(!close_tick(&mut app, true).contains(&ViewportCommand::Close));
            app.acknowledge_shutdown_failure();
            assert!(close_tick(&mut app, false).contains(&ViewportCommand::Close));
        } else {
            assert!(!staging.exists());
            assert!(commands.contains(&ViewportCommand::Close));
            if completed {
                let QueueJobState::Completed { output } =
                    app.core.queue.scheduler().jobs()[0].state()
                else {
                    panic!("completion was lost")
                };
                assert_eq!(fs::read(output).unwrap(), b"partial");
            } else {
                assert_eq!(
                    app.core.queue.scheduler().jobs()[0].state(),
                    &QueueJobState::Cancelled
                );
                assert!(!directory.join("existing (1).mkv").exists());
            }
        }
        assert!(!next.exists());
        assert_eq!(fs::read(output).unwrap(), b"original");
        drop(app);
        fs::remove_dir_all(directory).unwrap();
    }
}

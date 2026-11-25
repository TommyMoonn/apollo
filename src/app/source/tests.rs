use super::*;
use crate::media::{error::MediaErrorKind, probe::parse_ffprobe_json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(10);
static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        loop {
            let directory = std::env::temp_dir().join(format!(
                "apollo-source-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&directory) {
                Ok(()) => return Self(directory),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create source fixture: {error}"),
            }
        }
    }
    fn file(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, b"fixture media").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn media(path: &Path) -> MediaFile {
    parse_ffprobe_json(
        path,
        include_bytes!("../../../tests/fixtures/ffprobe/complete_inventory.json"),
    )
    .unwrap()
}
fn request(owner: &mut SourceSession, path: &Path) -> MediaIntakeRequest {
    match owner.begin_load(path) {
        Some(MediaLoadStart::Probe(request)) => request,
        other => panic!("expected new inspection, got {other:?}"),
    }
}
fn accept(owner: &mut SourceSession, path: &Path) {
    let pending = request(owner, path);
    assert_eq!(
        owner.complete_probe(pending, Ok(media(path))),
        SourceSettlement::Published
    );
}

#[test]
fn inspection_completion_notifies_owner_without_polling() {
    let fixture = Fixture::new();
    let path = fixture.file("video.mkv");
    let mut app = crate::app::Application::default();
    let pending = request(&mut app.source, &path);
    let (sender, receiver) = mpsc::sync_channel(1);
    let inspected = media(&path);
    app.source.start_inspection_with(
        pending,
        move || {
            let _ = sender.send(());
        },
        move || Ok(inspected),
    );
    receiver.recv_timeout(TIMEOUT).unwrap();
    assert!(app.poll_background(|| panic!("source inspection must not launch queue work")));
    assert_eq!(
        app.active_operation(),
        Some(crate::app::Operation::AudioMix)
    );
    assert_eq!(app.source.current().unwrap().path, path);
    assert!(!app.source.is_inspecting());
    assert!(!app.poll_background(|| panic!("idle queue must not launch")));
}

#[test]
fn inspection_worker_panic_notifies_owner_and_preserves_accepted_source() {
    let fixture = Fixture::new();
    let first = fixture.file("first.mkv");
    let second = fixture.file("second.mkv");
    let mut owner = SourceSession::default();
    accept(&mut owner, &first);
    let accepted_id = owner.session_id();
    let pending = request(&mut owner, &second);
    let (sender, receiver) = mpsc::sync_channel(1);
    owner.start_inspection_with(
        pending,
        move || {
            let _ = sender.send(());
        },
        || panic!("simulated inspection worker failure"),
    );
    receiver.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(owner.poll(), SourceSettlement::Failed);
    assert_eq!(
        owner.error().map(MediaError::kind),
        Some(MediaErrorKind::ProbeFailure)
    );
    assert_eq!(owner.current().unwrap().path, first);
    assert_eq!(owner.session_id(), accepted_id);
    assert!(!owner.is_inspecting());
    accept(&mut owner, &second);
    assert_ne!(owner.session_id(), accepted_id);
    assert!(owner.error().is_none());
}

#[test]
fn stale_completion_cannot_replace_newer_request_or_restored_source() {
    let fixture = Fixture::new();
    let first = fixture.file("first.mkv");
    let second = fixture.file("second.mkv");
    let mut owner = SourceSession::default();
    let old = request(&mut owner, &first);
    let newer = request(&mut owner, &second);
    assert_eq!(
        owner.complete_probe(old.clone(), Ok(media(&first))),
        SourceSettlement::Unchanged
    );
    assert!(owner.current().is_none());
    assert_eq!(
        owner.complete_probe(newer, Ok(media(&second))),
        SourceSettlement::Published
    );
    let previous_id = owner.session_id();
    let stale = request(&mut owner, &first);
    owner.restore(SourceIdentity::read(&second).unwrap(), media(&second));
    assert_ne!(owner.session_id(), previous_id);
    let restored_id = owner.session_id();
    let latest = request(&mut owner, &first);
    for retired in [old, stale] {
        assert_eq!(
            owner.complete_probe(retired, Ok(media(&first))),
            SourceSettlement::Unchanged
        );
        assert_eq!(owner.current().unwrap().path, second);
        assert_eq!(owner.session_id(), restored_id);
    }
    assert_eq!(
        owner.complete_probe(latest, Ok(media(&first))),
        SourceSettlement::Published
    );
    assert_eq!(owner.current().unwrap().path, first);
}

#[test]
fn close_retires_pending_worker_and_cannot_recreate_source() {
    let fixture = Fixture::new();
    let first = fixture.file("first.mkv");
    let second = fixture.file("second.mkv");
    let mut owner = SourceSession::default();
    accept(&mut owner, &first);
    let old_id = owner.session_id();
    let pending = request(&mut owner, &second);
    let (sender, receiver) = mpsc::sync_channel(1);
    owner.attach_inspection(pending.clone(), receiver);
    owner.close();
    assert!(!owner.is_inspecting());
    assert!(owner.current().is_none());
    assert!(owner.session_id().is_none());
    assert!(sender.send(Ok(media(&second))).is_err());
    assert_eq!(
        owner.complete_probe(pending, Ok(media(&second))),
        SourceSettlement::Unchanged
    );
    assert_eq!(owner.poll(), SourceSettlement::Unchanged);
    accept(&mut owner, &second);
    assert_ne!(owner.session_id(), old_id);
}

#[test]
fn unchanged_source_reuses_session_and_changed_source_requires_new_inspection() {
    let fixture = Fixture::new();
    let path = fixture.file("video.mkv");
    let mut owner = SourceSession::default();
    accept(&mut owner, &path);
    let original = owner.session_id();
    assert_eq!(owner.begin_load(&path), Some(MediaLoadStart::Reused));
    assert_eq!(owner.session_id(), original);
    fs::write(&path, b"different length").unwrap();
    let pending = request(&mut owner, &path);
    assert_eq!(owner.session_id(), original);
    assert_eq!(
        owner.complete_probe(pending, Ok(media(&path))),
        SourceSettlement::Published
    );
    assert_ne!(owner.session_id(), original);
}

fn ready_application(fixture: &Fixture) -> crate::app::Application {
    use crate::app::discovery::{MediaToolsDiscovery, MediaToolsState};
    crate::app::Application {
        discovery: MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
            fixture.0.join("absent-ffmpeg.exe"),
            fixture.0.join("absent-ffprobe.exe"),
        ))),
        ..Default::default()
    }
}

#[test]
fn external_paths_share_transactional_source_admission_and_preserve_queued_jobs() {
    use crate::{
        app::{NavigationState, Operation, dialogs::InputDialogOrigin},
        queue::QueuedOperation,
        remux::draft::RemuxDraft,
    };
    let fixture = Fixture::new();
    let original = fixture.file("first.mkv");
    let next = fixture.file("next.mkv");
    let mut app = ready_application(&fixture);
    let inspected = parse_ffprobe_json(
        &original,
        include_bytes!("../../../tests/fixtures/ffprobe/one_audio.json"),
    )
    .unwrap();
    app.source
        .restore(SourceIdentity::read(&original).unwrap(), inspected);
    app.enter_workspace(Operation::Remux);
    let (identity, source) = app.source.current_with_identity().unwrap();
    let job = app
        .queue
        .enqueue(
            identity,
            source,
            QueuedOperation::Remux(RemuxDraft::new(source).request(source)),
        )
        .unwrap();
    let origin = InputDialogOrigin::capture(&app);
    let id = app.source.session_id();
    app.accept_source_paths(origin, vec![original.clone()], || {
        panic!("unchanged source must not probe")
    });
    assert_eq!(app.source.session_id(), id);
    assert_eq!(
        app.settings.source_directory().as_deref(),
        Some(fixture.0.as_path())
    );
    app.accept_source_paths(origin, vec![fixture.0.clone()], || {
        panic!("directory must not probe")
    });
    assert_eq!(
        app.source.error().unwrap().kind(),
        MediaErrorKind::SourceUnavailable
    );
    assert_eq!(app.source.current().unwrap().path, original);
    let (sender, receiver) = mpsc::channel();
    app.accept_source_paths(origin, vec![next.clone()], move || {
        let _ = sender.send(());
    });
    assert!(app.source.is_inspecting());
    assert_eq!(app.source.current().unwrap().path, original);
    assert!(!app.can_accept_source_paths());
    receiver.recv_timeout(TIMEOUT).unwrap();
    assert!(app.poll_background(|| panic!("intake must not launch queue work")));
    assert_eq!(
        app.source.error().unwrap().kind(),
        MediaErrorKind::FfprobeUnavailable
    );
    assert_eq!(app.source.session_id(), id);
    assert_eq!(
        app.navigation(),
        NavigationState::SourceWorkspace {
            active_operation: Operation::Remux
        }
    );
    app.close_source();
    app.accept_source_paths(origin, vec![next], || {
        panic!("stale context must not probe")
    });
    assert_eq!(app.navigation(), NavigationState::NoSource);
    assert!(app.source.current().is_none());
    assert!(!app.source.is_inspecting());
    assert_eq!(app.queue.scheduler().jobs()[0].id(), job);
    assert_eq!(app.queue.scheduler().pending_count(), 1);
}

#[test]
fn multiple_external_paths_retain_batch_order_and_restore_previous_source_on_back() {
    use crate::app::{NavigationState, Operation, dialogs::InputDialogOrigin};
    let fixture = Fixture::new();
    let original = fixture.file("accepted.mkv");
    let paths = vec![fixture.file("first.mkv"), fixture.file("second.mp4")];
    let mut app = ready_application(&fixture);
    accept(&mut app.source, &original);
    app.enter_workspace(Operation::AudioExtract);
    let id = app.source.session_id();
    app.accept_source_paths(InputDialogOrigin::capture(&app), paths.clone(), || {});
    assert_eq!(app.navigation(), NavigationState::BatchSetup);
    let batch = app.batch_setup.as_ref().unwrap();
    assert_eq!(
        batch
            .items
            .iter()
            .map(|item| &item.path)
            .collect::<Vec<_>>(),
        paths.iter().collect::<Vec<_>>()
    );
    let origin = batch.origin;
    assert!(!app.can_accept_source_paths());
    app.return_from_batch(origin);
    assert_eq!(app.active_operation(), Some(Operation::AudioExtract));
    assert_eq!(app.source.current().unwrap().path, original);
    assert_eq!(app.source.session_id(), id);
    assert!(app.batch_setup.is_none());
    assert!(!app.batch_inspecting());
    assert!(app.queue.scheduler().jobs().is_empty());
}

#[test]
fn empty_or_invalid_external_selection_does_not_start_inspection() {
    use crate::app::dialogs::InputDialogOrigin;
    let fixture = Fixture::new();
    let missing = fixture.0.join("missing.mkv");
    let mut app = ready_application(&fixture);
    let origin = InputDialogOrigin::capture(&app);
    let no_probe = || panic!("invalid input must not start a worker");
    app.accept_source_paths(origin, vec![], no_probe);
    assert!(app.source.error().is_none());
    assert!(app.settings.source_directory().is_none());
    app.accept_source_paths(origin, vec![missing], no_probe);
    assert_eq!(
        app.source.error().unwrap().kind(),
        MediaErrorKind::SourceUnavailable
    );
    assert!(app.source.current().is_none());
    assert!(!app.source.is_inspecting());
    let path = fixture.file("valid.mkv");
    let mut unavailable = crate::app::Application::default();
    assert!(!unavailable.can_accept_source_paths());
    unavailable.accept_source_paths(
        InputDialogOrigin::capture(&unavailable),
        vec![path],
        no_probe,
    );
    assert!(unavailable.settings.source_directory().is_none());
    assert!(!unavailable.source.is_inspecting());
}

#[test]
fn source_completion_updates_the_return_origin_without_leaving_temporary_routes() {
    use crate::app::{NavigationState, Operation, ShellOrigin};
    let fixture = Fixture::new();
    let first = fixture.file("first.mkv");
    let second = fixture.file("second.mkv");
    for settings in [false, true] {
        for replacement in [false, true] {
            let mut app = ready_application(&fixture);
            let expected = if replacement {
                accept(&mut app.source, &first);
                app.enter_workspace(Operation::Remux);
                Operation::Remux
            } else {
                Operation::AudioMix
            };
            let pending = request(&mut app.source, &second);
            let (sender, receiver) = mpsc::channel();
            app.source.attach_inspection(pending, receiver);
            assert!(if settings {
                app.enter_settings()
            } else {
                app.enter_queue()
            });
            sender.send(Ok(media(&second))).unwrap();
            assert!(app.poll_background(|| panic!("navigation must not launch jobs")));
            let origin = ShellOrigin::SourceWorkspace {
                active_operation: expected,
            };
            assert_eq!(
                app.navigation(),
                if settings {
                    NavigationState::Settings { origin }
                } else {
                    NavigationState::Queue { origin }
                }
            );
            if settings {
                app.return_from_settings();
            } else {
                app.return_from_queue();
            }
            assert_eq!(app.active_operation(), Some(expected));
            assert_eq!(app.source.current().unwrap().path, second);
            let id = app.source.session_id();
            for operation in Operation::ALL {
                app.select_operation(operation);
                assert_eq!(app.active_operation(), Some(operation));
                assert_eq!(app.source.session_id(), id);
                assert_eq!(app.source.current().unwrap().path, second);
            }
        }
    }
}

#[test]
fn queue_add_files_reusing_source_keeps_queue_and_original_operation() {
    use crate::app::{
        NavigationState, Operation, QueueAction, ShellOrigin, dialogs::InputDialogOrigin,
        queue::QueueEffect,
    };
    let fixture = Fixture::new();
    let source = fixture.file("retained.mkv");
    let mut app = ready_application(&fixture);
    accept(&mut app.source, &source);
    app.enter_workspace(Operation::Remux);
    let session = app.source.session_id();
    assert!(app.enter_queue());
    assert!(app.can_accept_source_paths());
    assert!(matches!(
        app.queue_action(QueueAction::AddFiles, || panic!(
            "Add files must not launch queue work"
        )),
        QueueEffect::AddFiles
    ));
    let origin = InputDialogOrigin::capture(&app);
    app.accept_source_paths(origin, vec![source.clone()], || {
        panic!("unchanged source must not probe")
    });
    assert_eq!(
        app.navigation(),
        NavigationState::Queue {
            origin: ShellOrigin::SourceWorkspace {
                active_operation: Operation::Remux
            }
        }
    );
    assert_eq!(app.source.session_id(), session);
    assert!(!app.source.is_inspecting());
    app.return_from_queue();
    assert_eq!(app.active_operation(), Some(Operation::Remux));
    assert_eq!(app.source.current().unwrap().path, source);
}

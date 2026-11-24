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
    let mut owner = SourceSession::default();
    let pending = request(&mut owner, &path);
    let (sender, receiver) = mpsc::sync_channel(1);
    let inspected = media(&path);
    owner.start_inspection_with(
        pending,
        move || {
            let _ = sender.send(());
        },
        move || Ok(inspected),
    );
    receiver.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(owner.poll(), SourceSettlement::Published);
    assert_eq!(owner.current().unwrap().path, path);
    assert!(!owner.is_inspecting());
    assert_eq!(owner.poll(), SourceSettlement::Unchanged);
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

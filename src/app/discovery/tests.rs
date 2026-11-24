use super::*;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);

fn tools() -> MediaTools {
    MediaTools::new("ffmpeg", "ffprobe")
}

fn callback() -> (impl Fn() + Send + 'static, Receiver<()>) {
    let (sender, receiver) = mpsc::channel();
    (
        move || {
            let _ = sender.send(());
        },
        receiver,
    )
}

#[test]
fn progressive_discovery_wakes_for_base_and_capabilities_on_one_worker() {
    let mut owner = MediaToolsDiscovery::default();
    let (wake, notifications) = callback();
    let (release, gate) = mpsc::channel();
    let (base_thread, base_thread_id) = mpsc::channel();
    let (cap_thread, cap_thread_id) = mpsc::channel();
    let expected = VideoEncoderCapabilities {
        h264_nvenc: VideoEncoderCapability::Available,
        h265_nvenc: VideoEncoderCapability::RuntimeUnavailable {
            reason: "no HEVC".into(),
        },
    };
    let completed = expected.clone();
    assert!(owner.start_with(
        wake,
        move || {
            base_thread.send(thread::current().id()).unwrap();
            Ok(tools())
        },
        move |path| {
            assert_eq!(path, Path::new("ffmpeg"));
            cap_thread.send(thread::current().id()).unwrap();
            gate.recv_timeout(TIMEOUT).unwrap();
            completed
        }
    ));
    assert!(matches!(owner.state(), MediaToolsState::Checking));
    notifications.recv_timeout(TIMEOUT).unwrap();
    let worker = base_thread_id.recv_timeout(TIMEOUT).unwrap();
    assert_ne!(worker, thread::current().id());
    assert_eq!(cap_thread_id.recv_timeout(TIMEOUT).unwrap(), worker);
    assert!(!owner.poll());
    assert_eq!(owner.ready(), Some(&tools()));
    assert!(owner.is_pending());
    assert!(owner.settled_capabilities().is_none());
    assert!(!owner.start_with(
        || {},
        || panic!("overlapping discovery"),
        |_| unreachable!()
    ));
    release.send(()).unwrap();
    notifications.recv_timeout(TIMEOUT).unwrap();
    assert!(owner.poll());
    assert!(!owner.is_pending());
    assert_eq!(owner.settled_capabilities(), Some(&expected));
    assert!(!owner.poll());
}

#[test]
fn failed_discovery_notifies_and_retry_accepts_a_new_generation() {
    let mut owner = MediaToolsDiscovery::default();
    let (wake, notifications) = callback();
    assert!(owner.start_with(
        wake,
        || Err(MediaToolsError::MissingFfprobe),
        |_| unreachable!()
    ));
    notifications.recv_timeout(TIMEOUT).unwrap();
    assert!(owner.poll());
    assert_eq!(
        owner.state(),
        &MediaToolsState::Unavailable(MediaToolsError::MissingFfprobe)
    );
    assert!(!owner.is_pending());

    let (wake, notifications) = callback();
    assert!(owner.start_with(
        wake,
        || Ok(tools()),
        |_| VideoEncoderCapabilities::default()
    ));
    assert!(!owner.complete(0, Ok(MediaTools::new("stale-ffmpeg", "stale-ffprobe"))));
    notifications.recv_timeout(TIMEOUT).unwrap();
    notifications.recv_timeout(TIMEOUT).unwrap();
    assert!(owner.poll());
    assert_eq!(owner.ready(), Some(&tools()));
    assert!(!owner.complete(1, Err(MediaToolsError::MissingBoth)));
    assert_eq!(owner.ready(), Some(&tools()));
}

#[test]
fn stale_capability_attempt_or_path_cannot_replace_current_tools() {
    let mut owner = MediaToolsDiscovery::with_state(MediaToolsState::Ready(tools()));
    let (_sender, completion) = mpsc::sync_channel(1);
    owner.pending = Some(DiscoveryAttempt {
        id: 2,
        base: None,
        completion,
    });
    let current = tools();
    assert!(!owner.complete(1, Ok(current.clone())));
    assert!(!owner.complete(2, Ok(MediaTools::new("other-ffmpeg", "other-ffprobe"))));
    assert_eq!(owner.ready(), Some(&current));
    let mut settled = current;
    settled.video_encoders.h264_nvenc = VideoEncoderCapability::Available;
    assert!(owner.complete(2, Ok(settled.clone())));
    assert!(!owner.complete(2, Err(MediaToolsError::MissingBoth)));
    assert_eq!(owner.ready(), Some(&settled));
    assert!(!owner.is_pending());
}

#[test]
fn capability_disconnect_preserves_base_tools_and_retires_pending_state() {
    let mut owner = MediaToolsDiscovery::with_state(MediaToolsState::Ready(tools()));
    let (sender, completion) = mpsc::sync_channel(1);
    owner.pending = Some(DiscoveryAttempt {
        id: 0,
        base: None,
        completion,
    });
    drop(sender);
    assert!(owner.poll());
    let accepted = owner.ready().expect("base tools remain usable");
    assert_eq!(accepted.ffmpeg, tools().ffmpeg);
    assert_eq!(accepted.ffprobe, tools().ffprobe);
    assert!(matches!(
        accepted.video_encoders.h264_nvenc,
        VideoEncoderCapability::RuntimeUnavailable { .. }
    ));
    assert_eq!(
        accepted.video_encoders.h264_nvenc,
        accepted.video_encoders.h265_nvenc
    );
    assert!(!owner.is_pending());
}

#[test]
fn capability_panic_notifies_and_keeps_cpu_tools_ready() {
    let mut owner = MediaToolsDiscovery::default();
    let (wake, notifications) = callback();
    assert!(owner.start_with(wake, || Ok(tools()), |_| panic!("capability failure")));
    notifications.recv_timeout(TIMEOUT).unwrap();
    notifications.recv_timeout(TIMEOUT).unwrap();
    assert!(owner.poll());
    let accepted = owner.ready().unwrap();
    assert_eq!(accepted.ffmpeg, tools().ffmpeg);
    assert!(matches!(
        accepted.video_encoders.h264_nvenc,
        VideoEncoderCapability::RuntimeUnavailable { .. }
    ));
    assert_eq!(
        accepted.video_encoders.h264_nvenc,
        accepted.video_encoders.h265_nvenc
    );
    assert!(!owner.is_pending());
}

#[test]
fn disconnected_before_base_does_not_report_tools_ready() {
    let mut owner = MediaToolsDiscovery::with_state(MediaToolsState::Checking);
    let (sender, completion) = mpsc::sync_channel(1);
    owner.pending = Some(DiscoveryAttempt {
        id: 0,
        base: None,
        completion,
    });
    drop(sender);
    assert!(owner.poll());
    assert!(matches!(
        owner.state(),
        MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(_))
    ));
    assert!(!owner.is_pending());
}

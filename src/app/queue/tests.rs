use super::*;
use crate::media::process::MediaProcessEvent;
use crate::queue::QueueJobFailure;
use std::sync::Mutex;

#[test]
fn completed_queue_output_resolves_published_file_only_for_success() {
    let completed = QueueJobState::Completed {
        output: PathBuf::from("exports/output (1).mkv"),
    };
    assert_eq!(
        completed_output(&completed),
        Some(Path::new("exports/output (1).mkv"))
    );
    for state in [
        QueueJobState::Waiting,
        QueueJobState::Running {
            output: PathBuf::from("exports/staging.mkv"),
        },
        QueueJobState::Cancelled,
        QueueJobState::Failed(QueueJobFailure::SourceChanged),
    ] {
        assert!(completed_output(&state).is_none());
    }
}

#[test]
fn queue_wakes_are_observable_before_notification_and_retired_events_are_drained() {
    let first = JobId::new(1);
    let second = JobId::new(2);
    let (sender, receiver) = mpsc::channel();
    let receiver = Mutex::new(receiver);
    let wake = queue_execution_wake(
        first,
        sender,
        MediaProcessWake::new(move |event| {
            assert_eq!(event, MediaProcessEvent::Progress);
            assert_eq!(receiver.lock().unwrap().try_recv().unwrap().job_id, first);
        }),
    );
    wake.notify(MediaProcessEvent::Progress);
    let workflow = QueueWorkflow::default();
    let sender = workflow.sender.clone();
    let stale = queue_execution_wake(first, sender.clone(), MediaProcessWake::default());
    let current = queue_execution_wake(second, sender, MediaProcessWake::default());
    stale.notify(MediaProcessEvent::Terminal);
    assert!(!workflow.drain_current_events(Some(second)));
    current.notify(MediaProcessEvent::Terminal);
    stale.notify(MediaProcessEvent::Progress);
    assert!(workflow.drain_current_events(Some(second)));
    assert!(!workflow.drain_current_events(Some(first)));
    current.notify(MediaProcessEvent::Progress);
    assert!(!workflow.drain_current_events(None));
}

#[test]
fn output_action_failure_preserves_the_recorded_path_in_feedback() {
    use crate::native::OutputAction;
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("numbered (1).mkv");
    let mut app = Application::default();
    for action in [OutputAction::Open, OutputAction::Reveal] {
        assert!(!app.settle_output_action(action, &output, action.run(&output)));
        let message = app.queue_action_error.take().unwrap();
        assert!(message.contains(action.label()));
        assert!(message.contains(output.to_string_lossy().as_ref()));
        assert!(app.settle_output_action(action, &output, Ok(())));
        assert!(app.queue_action_error.is_none());
    }
}

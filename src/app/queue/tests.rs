use super::*;
use crate::media::process::MediaProcessEvent;
use crate::queue::QueueJobFailure;
use std::sync::Mutex;

#[test]
fn completed_queue_output_resolves_its_parent_folder_only_for_success() {
    let completed = QueueJobState::Completed {
        output: PathBuf::from("exports/output.mkv"),
    };
    assert_eq!(
        completed_output_folder(&completed),
        Some(Path::new("exports"))
    );
    assert_eq!(completed_output_folder(&QueueJobState::Waiting), None);
    assert_eq!(
        completed_output_folder(&QueueJobState::Failed(QueueJobFailure::SourceChanged)),
        None
    );
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

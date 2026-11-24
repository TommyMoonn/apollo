use super::*;

#[test]
fn queue_round_trip_preserves_the_origin_including_reentry() {
    let mut app = Application::default();
    app.enter_queue();
    assert_eq!(app.queue_origin(), Some(QueueOrigin::NoSource));
    app.return_from_queue();
    assert_eq!(app.navigation(), NavigationState::NoSource);

    app.enter_workspace(Operation::Remux);
    app.enter_queue();
    app.enter_queue();
    let origin = QueueOrigin::SourceWorkspace {
        active_operation: Operation::Remux,
    };
    assert_eq!(app.queue_origin(), Some(origin));
    assert!(app.can_enter_queue_edit(QueueEditOrigin::Queue(origin)));
    assert!(!app.can_enter_queue_edit(QueueEditOrigin::Queue(QueueOrigin::NoSource)));
    assert!(!app.can_enter_queue_edit(QueueEditOrigin::SourceWorkspace));
    app.enter_queue_edit(QueueEditOrigin::Queue(origin));
    app.enter_queue();
    app.return_from_queue();
    assert_eq!(app.active_operation(), Some(Operation::Remux));
}

#[test]
fn batch_cancel_and_enqueue_use_the_captured_return_origin() {
    let mut app = Application::default();
    for (route, origin, queue_origin) in [
        (
            NavigationState::NoSource,
            BatchOrigin::NoSource,
            QueueOrigin::NoSource,
        ),
        (
            NavigationState::SourceWorkspace {
                active_operation: Operation::AudioExtract,
            },
            BatchOrigin::SourceWorkspace {
                active_operation: Operation::AudioExtract,
            },
            QueueOrigin::SourceWorkspace {
                active_operation: Operation::AudioExtract,
            },
        ),
    ] {
        match route {
            NavigationState::NoSource => app.show_no_source(),
            NavigationState::SourceWorkspace { active_operation } => {
                app.enter_workspace(active_operation)
            }
            _ => unreachable!("fixture contains only source and intake origins"),
        }
        assert_eq!(app.batch_origin(), Some(origin));
        app.show_batch_setup();
        app.show_batch_edit();
        assert_eq!(app.batch_origin(), None);
        app.return_from_batch(origin);
        assert_eq!(app.navigation(), route);

        app.show_batch_setup();
        app.complete_batch(origin);
        assert_eq!(app.queue_origin(), Some(queue_origin));
        let from_queue = BatchOrigin::Queue {
            origin: queue_origin,
        };
        assert_eq!(app.batch_origin(), Some(from_queue));
        app.show_batch_setup();
        app.return_from_batch(from_queue);
        assert_eq!(app.queue_origin(), Some(queue_origin));

        app.enter_queue_edit(QueueEditOrigin::Queue(queue_origin));
        assert_eq!(app.batch_origin(), Some(from_queue));
        app.show_batch_setup();
        app.complete_batch(from_queue);
        assert_eq!(app.queue_origin(), Some(queue_origin));
        app.return_from_queue();
        assert_eq!(app.navigation(), route);
    }
}

#[test]
fn source_queue_editor_does_not_admit_a_batch_or_queue_origin() {
    let mut app = Application::default();
    assert!(!app.can_enter_queue_edit(QueueEditOrigin::SourceWorkspace));
    app.enter_workspace(Operation::FrameCapture);
    assert!(app.can_enter_queue_edit(QueueEditOrigin::SourceWorkspace));
    assert!(!app.can_enter_queue_edit(QueueEditOrigin::Queue(QueueOrigin::NoSource)));
    app.enter_queue_edit(QueueEditOrigin::SourceWorkspace);
    assert_eq!(app.batch_origin(), None);
    assert_eq!(app.active_operation(), None);
    app.close_source();
    assert_eq!(app.navigation(), NavigationState::NoSource);
}

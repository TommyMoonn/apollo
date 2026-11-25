//! Remux form routing. The accepted source and fixed queue editor own their drafts.
use super::{
    Application, NavigationState, Operation, QueueAction, dialogs::DialogOutcome,
    queue::QueueSubmission,
};
use crate::{
    media::{error::MediaError, model::MediaFile, process::MediaProcessWake},
    queue::QueuedOperation,
    remux::draft::RemuxDraft,
};

impl Application {
    pub(crate) fn remux_context(&self) -> Option<(u64, &MediaFile, &RemuxDraft)> {
        match self.navigation() {
            NavigationState::SourceWorkspace {
                active_operation: Operation::Remux,
            } => Some((
                self.source.session_id()?,
                self.source.current()?,
                self.source.remux.as_ref()?,
            )),
            NavigationState::QueueEdit { .. } => {
                let edit = self.queue_edit()?;
                (edit.active_operation == Operation::Remux).then_some((
                    edit.session_id,
                    &edit.media,
                    &edit.remux,
                ))
            }
            _ => None,
        }
    }

    pub(crate) fn remux_draft_mut(&mut self, session: u64) -> Option<&mut RemuxDraft> {
        if self.is_closing() || self.remux_context()?.0 != session || self.source.is_inspecting() {
            return None;
        }
        match self.navigation() {
            NavigationState::QueueEdit { .. } => {
                let edit = self.queue_edit_mut()?;
                edit.error = None;
                edit.error_details_open = false;
                Some(&mut edit.remux)
            }
            _ => {
                self.queue_confirmation = None;
                self.operation_action_error = None;
                self.source.remux.as_mut()
            }
        }
    }

    pub(crate) fn settle_source_remux_output(
        &mut self,
        session: u64,
        outcome: &DialogOutcome,
    ) -> bool {
        if self.active_operation() != Some(Operation::Remux)
            || self.remux_context().map(|context| context.0) != Some(session)
        {
            return false;
        }
        match outcome {
            DialogOutcome::Selected(path) => {
                let Some(draft) = self.remux_draft_mut(session) else {
                    return false;
                };
                draft.set_custom_output_path(path.clone());
                true
            }
            DialogOutcome::Failed(diagnostic) => {
                self.operation_action_error = Some((
                    Operation::Remux,
                    MediaError::output_picker_unavailable(diagnostic.clone()),
                ));
                false
            }
            _ => false,
        }
    }

    pub(crate) fn submit_remux(
        &mut self,
        session: u64,
        submission: QueueSubmission,
        wake: impl FnMut() -> MediaProcessWake,
    ) -> bool {
        if self.is_closing()
            || self.active_operation() != Some(Operation::Remux)
            || self.source.is_inspecting()
            || self.discovery.ready().is_none()
            || self.source.session_id() != Some(session)
            || (submission == QueueSubmission::ProcessNow && !self.process_now_allowed())
        {
            return false;
        }
        let Some((identity, media)) = self.source.current_with_identity() else {
            return false;
        };
        let Some(draft) = self.source.remux.as_ref() else {
            return false;
        };
        // Queue admission revalidates the request and destination, including source identity.
        let result = self
            .queue
            .enqueue(
                identity,
                media,
                QueuedOperation::Remux(draft.request(media)),
            )
            .map(Some);
        let admitted = result.is_ok();
        self.record_queue_submission(Operation::Remux, result);
        if admitted && submission == QueueSubmission::ProcessNow {
            self.enter_queue();
            self.queue_action(QueueAction::Start, wake);
        }
        admitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{
            QueueEditOrigin,
            dialogs::{
                DialogTarget, FileDialogCompletion, FileDialogRequest, InputDialogOrigin,
                OutputDraft,
            },
            discovery::{MediaToolsDiscovery, MediaToolsState},
        },
        media::{container::MediaContainer, intake::SourceIdentity, tools::MediaTools},
        queue::{QueueJobState, QueueRunState},
        remux::draft::tests::media,
    };
    use std::path::Path;

    fn ready_source() -> (tempfile::TempDir, Application, u64) {
        let root = tempfile::tempdir().unwrap();
        let mut media = media();
        media.path = root.path().join("source recording.mkv");
        std::fs::write(&media.path, b"fixture").unwrap();
        let mut app = Application {
            discovery: MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
                root.path().join("missing-ffmpeg"),
                root.path().join("missing-ffprobe"),
            ))),
            ..Application::default()
        };
        app.source
            .restore(SourceIdentity::read(&media.path).unwrap(), media);
        app.enter_workspace(Operation::Remux);
        let session = app.source.session_id().unwrap();
        (root, app, session)
    }

    #[test]
    fn normal_dialog_preserves_native_destination_and_rejects_retired_contexts() {
        let (root, mut app, session) = ready_source();
        let original = app.remux_context().unwrap().2.clone();
        let origin = InputDialogOrigin::capture(&app);
        let complete = |app: &mut Application, id, outcome| {
            app.settle_native_dialog(
                origin,
                FileDialogCompletion {
                    target: DialogTarget::RemuxOutput { session_id: id },
                    outcome,
                },
                || {},
            )
        };
        complete(&mut app, session, DialogOutcome::Cancelled);
        assert_eq!(&original, app.remux_context().unwrap().2);
        complete(
            &mut app,
            session,
            DialogOutcome::Failed("picker failed".into()),
        );
        assert_eq!(&original, app.remux_context().unwrap().2);
        assert!(app.operation_action_error.is_some());
        let selected = root.path().join("sortie vidéo 你好.mkv");
        complete(
            &mut app,
            session + 1,
            DialogOutcome::Selected(selected.clone()),
        );
        assert_eq!(&original, app.remux_context().unwrap().2);
        complete(&mut app, session, DialogOutcome::Selected(selected.clone()));
        assert_eq!(app.remux_context().unwrap().2.output_path(), selected);
        assert!(app.operation_action_error.is_none());
        assert_eq!(
            app.settings.output_directory(Path::new("missing")),
            Some(root.path().to_owned())
        );
        app.remux_draft_mut(session)
            .unwrap()
            .set_container(MediaContainer::Mp4);
        assert_eq!(
            app.remux_context().unwrap().2.output_path(),
            selected.with_extension("mp4")
        );
        assert!(app.select_operation(Operation::AudioMix));
        assert!(app.select_operation(Operation::Remux));
        complete(
            &mut app,
            session,
            DialogOutcome::Selected(root.path().join("stale.mkv")),
        );
        assert_eq!(
            app.remux_context().unwrap().2.output_path(),
            selected.with_extension("mp4")
        );
        app.close_source();
        assert!(app.remux_draft_mut(session).is_none());
        assert!(app.source.remux.is_none());
    }

    #[test]
    fn normal_and_queue_editor_share_admission_without_overwriting_the_original_draft() {
        let (root, mut app, session) = ready_source();
        let output = root.path().join("delivery copy.mkv");
        app.remux_draft_mut(session)
            .unwrap()
            .set_custom_output_path(output.clone());
        let never_launch = || panic!("stopped queue must not launch");
        assert!(app.submit_remux(session, QueueSubmission::AddToQueue, never_launch));
        assert_eq!(app.queue.scheduler().run_state(), QueueRunState::Stopped);
        let job = &app.queue.scheduler().jobs()[0];
        let id = job.id();
        let request = job.operation().clone();
        assert!(matches!(job.state(), QueueJobState::Waiting));
        assert!(!app.submit_remux(session, QueueSubmission::ProcessNow, never_launch));
        // Numbered normal submissions deliberately permit the same requested destination.
        assert!(app.submit_remux(session, QueueSubmission::AddToQueue, never_launch));
        assert_eq!(app.queue.scheduler().jobs().len(), 2);
        app.remux_draft_mut(session)
            .unwrap()
            .set_output_filename_stem("CON".into());
        assert!(!app.submit_remux(session, QueueSubmission::AddToQueue, never_launch));
        assert!(app.operation_action_error.is_some());
        app.remux_draft_mut(session)
            .unwrap()
            .set_custom_output_path(output.clone());
        assert_eq!(app.remux_context().unwrap().2.output_path(), output);
        assert!(app.enter_queue());
        let origin = QueueEditOrigin::Queue(app.queue_origin().unwrap());
        let edit = app.prepare_queue_edit(id, origin).unwrap();
        app.activate_queue_edit(edit, origin);
        let (edit_session, media, draft) = app.remux_context().unwrap();
        assert_ne!(session, edit_session);
        assert_eq!(QueuedOperation::Remux(draft.request(media)), request);
        assert!(app.remux_draft_mut(session).is_none());
        app.remux_draft_mut(edit_session)
            .unwrap()
            .set_output_filename_stem("CON".into());
        assert!(!app.save_queue_edit());
        assert_eq!(app.queue.scheduler().jobs()[0].operation(), &request);
        assert_eq!(app.remux_context().unwrap().2.output_filename_stem(), "CON");
        app.remux_draft_mut(edit_session)
            .unwrap()
            .set_output_filename_stem("revised".into());
        app.remux_draft_mut(edit_session)
            .unwrap()
            .set_container(MediaContainer::Mp4);
        assert!(app.save_queue_edit());
        assert_eq!(app.queue.scheduler().jobs()[0].id(), id);
        assert_eq!(
            app.queue.scheduler().jobs()[0].operation().output(),
            root.path().join("revised.mp4")
        );
        assert!(app.finish_queue_edit().is_some());
        assert!(app.remux_draft_mut(edit_session).is_none());
        let edit = app.prepare_queue_edit(id, origin).unwrap();
        app.activate_queue_edit(edit, origin);
        let cancel_session = app.remux_context().unwrap().0;
        app.remux_draft_mut(cancel_session)
            .unwrap()
            .set_output_filename_stem("discard".into());
        assert!(app.finish_queue_edit().is_some());
        assert_eq!(
            app.queue.scheduler().jobs()[0].operation().output(),
            root.path().join("revised.mp4")
        );
        assert!(app.remux_draft_mut(cancel_session).is_none());
        app.return_from_queue();
        assert_eq!(app.remux_context().unwrap().2.output_path(), output);
    }

    #[test]
    fn immediate_submission_enters_queue_even_with_finished_history() {
        let (_root, mut app, session) = ready_source();
        assert!(app.submit_remux(
            session,
            QueueSubmission::ProcessNow,
            MediaProcessWake::default
        ));
        assert!(matches!(app.navigation(), NavigationState::Queue { .. }));
        // The deliberately absent executable makes launch fail synchronously. It must
        // leave a terminal job and allow another immediate submission, not lose the request.
        assert!(matches!(
            app.queue.scheduler().jobs()[0].state(),
            QueueJobState::Failed(_)
        ));
        app.return_from_queue();
        assert!(app.process_now_allowed());
        assert!(app.submit_remux(
            session,
            QueueSubmission::ProcessNow,
            MediaProcessWake::default
        ));
        assert_eq!(app.queue.scheduler().jobs().len(), 2);
        assert!(matches!(app.navigation(), NavigationState::Queue { .. }));
    }

    #[test]
    fn invalid_name_can_recover_through_browse_in_the_retained_directory() {
        let (root, mut app, session) = ready_source();
        let draft = app.remux_draft_mut(session).unwrap();
        draft.set_custom_output_path(root.path().join("custom/name.mkv"));
        draft.set_output_filename_stem("CON".into());
        let FileDialogRequest::SaveOutput { options, .. } =
            OutputDraft::Remux(draft).request(session)
        else {
            panic!("expected save dialog");
        };
        assert_eq!(options.initial_directory, Some(root.path().join("custom")));
        assert!(options.file_name.is_empty());
    }
}

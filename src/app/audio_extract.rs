//! Session-scoped extraction draft routing for source and waiting-job editors.
use super::{Application, NavigationState, Operation};
use crate::{audio_extract::draft::AudioExtractDraft, media::model::MediaFile};

impl Application {
    pub(crate) fn audio_extract_context(&self) -> Option<(u64, &MediaFile, &AudioExtractDraft)> {
        match self.navigation() {
            NavigationState::SourceWorkspace {
                active_operation: Operation::AudioExtract,
            } => Some((
                self.source.session_id()?,
                self.source.current()?,
                self.source.audio_extract.as_ref()?,
            )),
            NavigationState::QueueEdit { .. } => {
                let edit = self.queue_edit()?;
                (edit.active_operation == Operation::AudioExtract).then_some((
                    edit.session_id,
                    &edit.media,
                    &edit.audio_extract,
                ))
            }
            _ => None,
        }
    }

    pub(crate) fn audio_extract_draft_mut(
        &mut self,
        session: u64,
    ) -> Option<(&MediaFile, &mut AudioExtractDraft)> {
        if self.is_closing()
            || self.source.is_inspecting()
            || self.audio_extract_context()?.0 != session
        {
            return None;
        }
        if matches!(self.navigation(), NavigationState::QueueEdit { .. }) {
            let edit = self.queue_edit_mut()?;
            edit.editor.error = None;
            edit.editor.error_details_open = false;
            return Some((&edit.editor.media, &mut edit.editor.audio_extract));
        }
        self.queue_confirmation = None;
        self.operation_action_error = None;
        self.source.audio_extract_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::queue::QueueSubmission, media::process::MediaProcessWake, queue::QueuedOperation,
    };
    use crate::{
        app::{
            QueueEditOrigin,
            dialogs::{
                DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogRequest,
                InputDialogOrigin, OutputDraft,
            },
            discovery::{MediaToolsDiscovery, MediaToolsState},
        },
        audio_extract::draft::tests::media,
        audio_extract::model::AudioExtractFormat,
        media::{intake::SourceIdentity, tools::MediaTools},
        queue::{QueueJobState, QueueRunState},
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
        app.enter_workspace(Operation::AudioExtract);
        let session = app.source.session_id().unwrap();
        (root, app, session)
    }

    #[test]
    fn normal_dialog_preserves_native_destination_and_rejects_retired_contexts() {
        let (root, mut app, session) = ready_source();
        let original = app.audio_extract_context().unwrap().2.clone();
        let origin = InputDialogOrigin::capture(&app);
        let complete = |app: &mut Application, id, outcome| {
            app.settle_native_dialog(
                origin,
                FileDialogCompletion {
                    target: DialogTarget::AudioExtractOutput { session_id: id },
                    outcome,
                },
                || {},
            )
        };
        complete(&mut app, session, DialogOutcome::Cancelled);
        assert_eq!(&original, app.audio_extract_context().unwrap().2);
        complete(
            &mut app,
            session,
            DialogOutcome::Failed("picker failed".into()),
        );
        assert_eq!(&original, app.audio_extract_context().unwrap().2);
        assert!(app.operation_action_error.is_some());
        let selected = root.path().join("sortie vidéo 你好.mka");
        complete(
            &mut app,
            session + 1,
            DialogOutcome::Selected(selected.clone()),
        );
        assert_eq!(&original, app.audio_extract_context().unwrap().2);
        complete(&mut app, session, DialogOutcome::Selected(selected.clone()));
        assert_eq!(
            app.audio_extract_context().unwrap().2.output_path(),
            selected
        );
        assert!(app.operation_action_error.is_none());
        assert_eq!(
            app.settings.output_directory(Path::new("missing")),
            Some(root.path().to_owned())
        );
        app.audio_extract_draft_mut(session)
            .unwrap()
            .1
            .set_format(AudioExtractFormat::Flac);
        assert_eq!(
            app.audio_extract_context().unwrap().2.output_path(),
            selected.with_extension("flac")
        );
        assert!(app.select_operation(Operation::AudioMix));
        assert!(app.select_operation(Operation::AudioExtract));
        complete(
            &mut app,
            session,
            DialogOutcome::Selected(root.path().join("stale.mka")),
        );
        assert_eq!(
            app.audio_extract_context().unwrap().2.output_path(),
            selected.with_extension("flac")
        );
        app.close_source();
        assert!(app.audio_extract_draft_mut(session).is_none());
        assert!(app.source.audio_extract.is_none());
    }

    #[test]
    fn normal_and_queue_editor_share_admission_without_overwriting_the_original_draft() {
        let (root, mut app, session) = ready_source();
        let output = root.path().join("delivery copy.mka");
        app.audio_extract_draft_mut(session)
            .unwrap()
            .1
            .set_custom_output_path(output.clone());
        let never_launch = || panic!("stopped queue must not launch");
        assert!(app.submit_operation(
            Operation::AudioExtract,
            session,
            QueueSubmission::AddToQueue,
            never_launch
        ));
        assert_eq!(app.queue.scheduler().run_state(), QueueRunState::Stopped);
        let job = &app.queue.scheduler().jobs()[0];
        let id = job.id();
        let request = job.operation().clone();
        assert!(matches!(job.state(), QueueJobState::Waiting));
        assert!(!app.submit_operation(
            Operation::AudioExtract,
            session,
            QueueSubmission::ProcessNow,
            never_launch
        ));
        // Numbered normal submissions deliberately permit the same requested destination.
        assert!(app.submit_operation(
            Operation::AudioExtract,
            session,
            QueueSubmission::AddToQueue,
            never_launch
        ));
        assert_eq!(app.queue.scheduler().jobs().len(), 2);
        app.audio_extract_draft_mut(session)
            .unwrap()
            .1
            .set_output_filename_stem("CON".into());
        assert!(!app.submit_operation(
            Operation::AudioExtract,
            session,
            QueueSubmission::AddToQueue,
            never_launch
        ));
        assert!(app.operation_action_error.is_some());
        app.audio_extract_draft_mut(session)
            .unwrap()
            .1
            .set_custom_output_path(output.clone());
        assert_eq!(app.audio_extract_context().unwrap().2.output_path(), output);
        assert!(app.enter_queue());
        let origin = QueueEditOrigin::Queue(app.queue_origin().unwrap());
        let edit = app.prepare_queue_edit(id, origin).unwrap();
        app.activate_queue_edit(edit, origin);
        let (edit_session, media, draft) = app.audio_extract_context().unwrap();
        assert_eq!(draft.selected_stream_index(), Some(3));
        assert_ne!(session, edit_session);
        assert_eq!(
            QueuedOperation::AudioExtract(draft.request(media).unwrap()),
            request
        );
        assert!(app.audio_extract_draft_mut(session).is_none());
        app.audio_extract_draft_mut(edit_session)
            .unwrap()
            .1
            .set_output_filename_stem("CON".into());
        assert!(!app.save_queue_edit());
        assert_eq!(app.queue.scheduler().jobs()[0].operation(), &request);
        assert_eq!(
            app.audio_extract_context()
                .unwrap()
                .2
                .output_filename_stem(),
            "CON"
        );
        // A second waiting job owns this destination; an edit must not claim it.
        app.audio_extract_draft_mut(edit_session)
            .unwrap()
            .1
            .set_output_filename_stem("delivery copy".into());
        assert!(!app.save_queue_edit());
        assert_eq!(app.queue.scheduler().jobs()[0].operation(), &request);
        assert_eq!(app.queue.scheduler().jobs()[0].id(), id);
        app.audio_extract_draft_mut(edit_session)
            .unwrap()
            .1
            .set_output_filename_stem("revised".into());
        app.audio_extract_draft_mut(edit_session)
            .unwrap()
            .1
            .set_format(AudioExtractFormat::Flac);
        {
            let (media, draft) = app.audio_extract_draft_mut(edit_session).unwrap();
            assert!(draft.select_stream(media, 7));
        }
        assert!(app.save_queue_edit());
        assert!(
            matches!(app.queue.scheduler().jobs()[0].operation(), QueuedOperation::AudioExtract(request) if request.audio_stream_index == 7)
        );
        assert_eq!(app.queue.scheduler().jobs()[0].id(), id);
        assert_eq!(
            app.queue.scheduler().jobs()[0].operation().output(),
            root.path().join("revised.flac")
        );
        assert!(app.finish_queue_edit().is_some());
        assert!(app.audio_extract_draft_mut(edit_session).is_none());
        let edit = app.prepare_queue_edit(id, origin).unwrap();
        app.activate_queue_edit(edit, origin);
        let cancel_session = app.audio_extract_context().unwrap().0;
        app.audio_extract_draft_mut(cancel_session)
            .unwrap()
            .1
            .set_output_filename_stem("discard".into());
        assert!(app.finish_queue_edit().is_some());
        assert_eq!(
            app.queue.scheduler().jobs()[0].operation().output(),
            root.path().join("revised.flac")
        );
        assert!(app.audio_extract_draft_mut(cancel_session).is_none());
        app.return_from_queue();
        assert_eq!(app.audio_extract_context().unwrap().2.output_path(), output);
    }

    #[test]
    fn immediate_submission_enters_queue_even_with_finished_history() {
        let (_root, mut app, session) = ready_source();
        assert!(app.submit_operation(
            Operation::AudioExtract,
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
        assert!(app.submit_operation(
            Operation::AudioExtract,
            session,
            QueueSubmission::ProcessNow,
            MediaProcessWake::default
        ));
        assert_eq!(app.queue.scheduler().jobs().len(), 2);
        assert!(matches!(app.navigation(), NavigationState::Queue { .. }));
    }

    #[test]
    fn no_audio_source_cannot_submit_or_select_video_stream() {
        let (_root, mut app, _) = ready_source();
        let mut media = app.source.current().unwrap().clone();
        media.audio_streams.clear();
        media
            .streams
            .retain(|stream| stream.kind != crate::media::model::StreamKind::Audio);
        app.source
            .restore(SourceIdentity::read(&media.path).unwrap(), media);
        let session = app.source.session_id().unwrap();
        let (media, draft) = app.audio_extract_draft_mut(session).unwrap();
        assert!(!draft.select_stream(media, 0));
        assert!(!draft.can_extract(media));
        assert!(!app.submit_operation(
            Operation::AudioExtract,
            session,
            QueueSubmission::AddToQueue,
            || panic!("no audio job may launch")
        ));
        assert!(app.queue.scheduler().jobs().is_empty());
    }

    #[test]
    fn invalid_filename_keeps_browse_directory_and_format() {
        let (root, mut app, session) = ready_source();
        let (_, draft) = app.audio_extract_draft_mut(session).unwrap();
        draft.set_custom_output_path(root.path().join("custom/name.mka"));
        draft.set_format(AudioExtractFormat::Aac);
        draft.set_output_filename_stem("CON".into());
        let FileDialogRequest::SaveOutput { options, .. } =
            OutputDraft::AudioExtract(draft).request(session)
        else {
            panic!("expected save dialog");
        };
        assert_eq!(options.initial_directory, Some(root.path().join("custom")));
        assert_eq!(options.format, crate::app::dialogs::OutputFormat::M4a);
        assert!(options.file_name.is_empty());
    }
}

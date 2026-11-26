//! Session-scoped mixer draft mutations for source and queue editor routes.
use super::{Application, NavigationState, Operation};
use crate::{
    audio_mix::{
        draft::AudioMixDraft,
        model::{LoudnessTargets, OutputContainer, TrackGain},
    },
    media::model::MediaFile,
};

#[derive(Clone, Copy)]
pub(crate) enum MixerAction {
    Toggle(u32),
    SelectAll,
    Clear,
    Gain(u32, TrackGain),
    ResetGains,
    Normalize(bool),
    Targets(LoudnessTargets),
    ResetTargets,
    Container(OutputContainer),
}

impl Application {
    pub(crate) fn audio_mix_context(&self) -> Option<(u64, &MediaFile, &AudioMixDraft)> {
        match self.navigation() {
            NavigationState::SourceWorkspace {
                active_operation: Operation::AudioMix,
            } => Some((
                self.source.session_id()?,
                self.source.current()?,
                self.source.audio_mix.as_ref()?,
            )),
            NavigationState::QueueEdit { .. } => {
                let edit = self.queue_edit()?;
                (edit.active_operation == Operation::AudioMix).then_some((
                    edit.session_id,
                    &edit.media,
                    &edit.audio_mix,
                ))
            }
            _ => None,
        }
    }

    pub(crate) fn audio_mix_draft_mut(
        &mut self,
        session: u64,
    ) -> Option<(&MediaFile, &mut AudioMixDraft)> {
        if self.is_closing()
            || self.source.is_inspecting()
            || self.audio_mix_context()?.0 != session
        {
            return None;
        }
        if matches!(self.navigation(), NavigationState::QueueEdit { .. }) {
            let edit = self.queue_edit_mut()?;
            edit.editor.error = None;
            edit.editor.error_details_open = false;
            return Some((&edit.editor.media, &mut edit.editor.audio_mix));
        }
        self.queue_confirmation = None;
        self.operation_action_error = None;
        self.source.audio_mix_mut()
    }

    /// Readiness is cached on the typed draft; rendering must not invalidate it.
    pub(crate) fn audio_mix_validation(
        &mut self,
    ) -> Option<Result<(), crate::audio_mix::validation::AudioMixValidationError>> {
        self.audio_mix_context()?;
        if matches!(self.navigation(), NavigationState::QueueEdit { .. }) {
            let edit = &mut self.queue_edit_mut()?.editor;
            return Some(edit.audio_mix.validation(&edit.media).clone());
        }
        let (media, draft) = self.source.audio_mix_mut()?;
        Some(draft.validation(media).clone())
    }

    pub(crate) fn edit_audio_mix(&mut self, session: u64, action: MixerAction) -> bool {
        let Some((media, draft)) = self.audio_mix_draft_mut(session) else {
            return false;
        };
        match action {
            MixerAction::Toggle(id) => draft.toggle_stream(media, id),
            MixerAction::Gain(id, gain) => draft.set_track_gain(media, id, gain),
            MixerAction::SelectAll => {
                draft.select_all(media);
                true
            }
            MixerAction::Clear => {
                draft.clear();
                true
            }
            MixerAction::Normalize(enabled) => {
                draft.set_normalization_enabled(enabled);
                true
            }
            MixerAction::Targets(targets) => {
                draft.normalization_enabled() && draft.set_normalization_targets(targets)
            }
            MixerAction::ResetTargets => {
                draft.reset_normalization_targets();
                true
            }
            MixerAction::Container(container) => {
                draft.set_container(media, container);
                true
            }
            MixerAction::ResetGains => {
                draft.reset_track_gains();
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audio_mix::draft::tests::media, media::intake::SourceIdentity};

    #[test]
    fn mixer_normalization_and_output_survive_queue_edit_and_cancel() {
        use crate::app::{
            QueueEditOrigin,
            dialogs::{DialogOutcome, DialogTarget, FileDialogCompletion, InputDialogOrigin},
            discovery::{MediaToolsDiscovery, MediaToolsState},
            queue::QueueSubmission,
        };
        use crate::{
            audio_mix::model::LoudnessNormalization, media::tools::MediaTools,
            queue::QueuedOperation,
        };
        let dir = tempfile::tempdir().unwrap();
        let mut source = media();
        source.path = dir.path().join("source.mkv");
        std::fs::write(&source.path, b"fixture").unwrap();
        let mut app = Application {
            discovery: MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
                dir.path().join("ffmpeg"),
                dir.path().join("ffprobe"),
            ))),
            ..Application::default()
        };
        app.source
            .restore(SourceIdentity::read(&source.path).unwrap(), source);
        app.enter_workspace(Operation::AudioMix);
        let session = app.audio_mix_context().unwrap().0;
        let targets = LoudnessTargets::from_tenths(-205, -12, 75);
        assert!(!app.edit_audio_mix(session, MixerAction::Targets(targets)));
        assert!(app.edit_audio_mix(session, MixerAction::Normalize(true)));
        assert!(app.edit_audio_mix(session, MixerAction::Targets(targets)));
        assert!(app.edit_audio_mix(session, MixerAction::Normalize(false)));
        assert_eq!(
            app.audio_mix_context().unwrap().2.normalization_targets(),
            targets
        );
        assert!(app.edit_audio_mix(session, MixerAction::Normalize(true)));
        assert!(app.edit_audio_mix(session, MixerAction::Container(OutputContainer::Mkv)));
        let path = dir.path().join("custom mix.mkv");
        let origin = InputDialogOrigin::capture(&app);
        app.settle_native_dialog(
            origin,
            FileDialogCompletion {
                target: DialogTarget::AudioMixOutput {
                    session_id: session,
                },
                outcome: DialogOutcome::Selected(path.clone()),
            },
            || {},
        );
        assert_eq!(app.audio_mix_context().unwrap().2.output_path(), path);
        assert!(app.audio_mix_validation().unwrap().is_ok());
        let never_launch = || panic!("stopped queue must not launch");
        assert!(app.submit_operation(
            Operation::AudioMix,
            session,
            QueueSubmission::AddToQueue,
            never_launch
        ));
        assert!(!app.submit_operation(
            Operation::AudioMix,
            session,
            QueueSubmission::ProcessNow,
            never_launch
        ));
        let job = &app.queue.scheduler().jobs()[0];
        let id = job.id();
        let original = job.operation().clone();
        assert!(
            matches!(&original, QueuedOperation::AudioMix(request) if request.normalization == LoudnessNormalization::Enabled(targets))
        );
        assert!(app.enter_queue());
        let origin = QueueEditOrigin::Queue(app.queue_origin().unwrap());
        let editor = app.prepare_queue_edit(id, origin).unwrap();
        app.activate_queue_edit(editor, origin);
        let (edit_session, media, draft) = app.audio_mix_context().unwrap();
        assert_ne!(session, edit_session);
        assert_eq!(QueuedOperation::AudioMix(draft.request(media)), original);
        assert!(!app.edit_audio_mix(session, MixerAction::ResetTargets));
        assert!(app.edit_audio_mix(edit_session, MixerAction::ResetTargets));
        app.finish_queue_edit();
        assert_eq!(app.queue.scheduler().jobs()[0].operation(), &original);
        let editor = app.prepare_queue_edit(id, origin).unwrap();
        app.activate_queue_edit(editor, origin);
        let edit_session = app.audio_mix_context().unwrap().0;
        assert!(app.edit_audio_mix(edit_session, MixerAction::ResetTargets));
        app.audio_mix_draft_mut(edit_session)
            .unwrap()
            .1
            .set_output_filename_stem("CON".into());
        assert!(!app.save_queue_edit());
        assert_eq!(app.queue.scheduler().jobs()[0].operation(), &original);
        app.audio_mix_draft_mut(edit_session)
            .unwrap()
            .1
            .set_output_filename_stem("revised".into());
        assert!(app.save_queue_edit());
        let job = &app.queue.scheduler().jobs()[0];
        assert_eq!(job.id(), id);
        assert!(
            matches!(job.operation(), QueuedOperation::AudioMix(request) if request.normalization == LoudnessNormalization::Enabled(LoudnessTargets::default()) && request.output == path.with_file_name("revised.mkv"))
        );
        app.finish_queue_edit();
        app.return_from_queue();
        let (_, _, draft) = app.audio_mix_context().unwrap();
        assert_eq!(draft.normalization_targets(), targets);
        assert_eq!(draft.output_path(), path);
    }

    #[test]
    fn empty_and_single_track_sources_have_fresh_drafts() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = media();
        source.path = dir.path().join("empty.mkv");
        std::fs::write(&source.path, b"fixture").unwrap();
        let mut app = Application::default();
        let track = source.audio_streams.pop().unwrap();
        source.audio_streams.clear();
        app.source
            .restore(SourceIdentity::read(&source.path).unwrap(), source.clone());
        app.enter_workspace(Operation::AudioMix);
        let session = app.audio_mix_context().unwrap().0;
        assert!(app.edit_audio_mix(session, MixerAction::SelectAll));
        assert_eq!(app.audio_mix_context().unwrap().2.selected_count(), 0);
        assert!(!app.edit_audio_mix(session, MixerAction::Toggle(track.stream_index)));
        source.path = dir.path().join("single.mkv");
        std::fs::write(&source.path, b"fixture").unwrap();
        let id = track.stream_index;
        source.audio_streams.push(track);
        app.source
            .restore(SourceIdentity::read(&source.path).unwrap(), source);
        let (new_session, _, draft) = app.audio_mix_context().unwrap();
        assert_ne!(session, new_session);
        assert_eq!(draft.selected_count(), 1);
        assert!(draft.is_selected(id));
        assert_eq!(draft.track_gain(id), Some(TrackGain::ZERO));
        assert!(!app.edit_audio_mix(session, MixerAction::Clear));
    }

    #[test]
    fn track_edits_follow_the_accepted_source_and_preserve_other_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = media();
        source.path = dir.path().join("source.mkv");
        std::fs::write(&source.path, b"fixture").unwrap();
        let mut app = Application::default();
        app.source
            .restore(SourceIdentity::read(&source.path).unwrap(), source.clone());
        app.enter_workspace(Operation::AudioMix);
        let session = app.audio_mix_context().unwrap().0;
        let id = source.audio_streams[0].stream_index;
        let other = source.audio_streams[1].stream_index;
        let gain = TrackGain::from_tenths_db(-125);
        assert!(app.edit_audio_mix(session, MixerAction::Toggle(id)));
        assert!(app.edit_audio_mix(session, MixerAction::Gain(id, gain)));
        assert!(!app.edit_audio_mix(session, MixerAction::Gain(u32::MAX, gain)));
        assert!(app.edit_audio_mix(session, MixerAction::Clear));
        assert_eq!(app.audio_mix_context().unwrap().2.selected_count(), 0);
        assert!(app.edit_audio_mix(session, MixerAction::SelectAll));
        assert!(app.select_operation(Operation::Remux));
        assert!(!app.edit_audio_mix(session, MixerAction::ResetGains));
        assert!(app.select_operation(Operation::AudioMix));
        let (_, media, draft) = app.audio_mix_context().unwrap();
        assert_eq!(draft.track_gain(id), Some(gain));
        assert_eq!(draft.track_gain(other), Some(TrackGain::ZERO));
        assert!(
            draft
                .request(media)
                .selected_tracks
                .iter()
                .any(|track| track.stream_index == id && track.gain == gain)
        );
        assert!(app.edit_audio_mix(session, MixerAction::ResetGains));
        let (_, _, draft) = app.audio_mix_context().unwrap();
        assert_eq!(draft.track_gain(id), Some(TrackGain::ZERO));
        assert_eq!(draft.selected_count(), source.audio_streams.len());
        app.close_source();
        assert!(!app.edit_audio_mix(session, MixerAction::Clear));
        app.source
            .restore(SourceIdentity::read(&source.path).unwrap(), source);
        app.enter_workspace(Operation::AudioMix);
        assert_ne!(app.audio_mix_context().unwrap().0, session);
        assert!(!app.edit_audio_mix(session, MixerAction::Clear));
    }
}

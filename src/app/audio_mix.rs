//! Session-scoped mixer mutations. Submission remains gated until the full form is ported.
use super::{Application, NavigationState, Operation};
use crate::{
    audio_mix::{draft::AudioMixDraft, model::TrackGain},
    media::model::MediaFile,
};

#[derive(Clone, Copy)]
pub(crate) enum TrackAction {
    Toggle(u32),
    SelectAll,
    Clear,
    Gain(u32, TrackGain),
    ResetGains,
}

impl Application {
    pub(crate) fn audio_mix_context(&self) -> Option<(u64, &MediaFile, &AudioMixDraft)> {
        if self.navigation()
            != (NavigationState::SourceWorkspace {
                active_operation: Operation::AudioMix,
            })
        {
            return None;
        }
        Some((
            self.source.session_id()?,
            self.source.current()?,
            self.source.audio_mix.as_ref()?,
        ))
    }

    pub(crate) fn edit_audio_mix_tracks(&mut self, session: u64, action: TrackAction) -> bool {
        if self.is_closing()
            || self.source.is_inspecting()
            || self.audio_mix_context().map(|context| context.0) != Some(session)
        {
            return false;
        }
        let Some((media, draft)) = self.source.audio_mix_mut() else {
            return false;
        };
        match action {
            TrackAction::Toggle(id) => draft.toggle_stream(media, id),
            TrackAction::Gain(id, gain) => draft.set_track_gain(media, id, gain),
            TrackAction::SelectAll => {
                draft.select_all(media);
                true
            }
            TrackAction::Clear => {
                draft.clear();
                true
            }
            TrackAction::ResetGains => {
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
        assert!(app.edit_audio_mix_tracks(session, TrackAction::SelectAll));
        assert_eq!(app.audio_mix_context().unwrap().2.selected_count(), 0);
        assert!(!app.edit_audio_mix_tracks(session, TrackAction::Toggle(track.stream_index)));
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
        assert!(!app.edit_audio_mix_tracks(session, TrackAction::Clear));
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
        assert!(app.edit_audio_mix_tracks(session, TrackAction::Toggle(id)));
        assert!(app.edit_audio_mix_tracks(session, TrackAction::Gain(id, gain)));
        assert!(!app.edit_audio_mix_tracks(session, TrackAction::Gain(u32::MAX, gain)));
        assert!(app.edit_audio_mix_tracks(session, TrackAction::Clear));
        assert_eq!(app.audio_mix_context().unwrap().2.selected_count(), 0);
        assert!(app.edit_audio_mix_tracks(session, TrackAction::SelectAll));
        assert!(app.select_operation(Operation::Remux));
        assert!(!app.edit_audio_mix_tracks(session, TrackAction::ResetGains));
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
        assert!(app.edit_audio_mix_tracks(session, TrackAction::ResetGains));
        let (_, _, draft) = app.audio_mix_context().unwrap();
        assert_eq!(draft.track_gain(id), Some(TrackGain::ZERO));
        assert_eq!(draft.selected_count(), source.audio_streams.len());
        app.close_source();
        assert!(!app.edit_audio_mix_tracks(session, TrackAction::Clear));
        app.source
            .restore(SourceIdentity::read(&source.path).unwrap(), source);
        app.enter_workspace(Operation::AudioMix);
        assert_ne!(app.audio_mix_context().unwrap().0, session);
        assert!(!app.edit_audio_mix_tracks(session, TrackAction::Clear));
    }
}

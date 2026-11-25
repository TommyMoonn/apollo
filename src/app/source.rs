//! Application source session and asynchronous inspection. MediaIntake remains the probe authority.
use crate::media::{
    error::MediaError,
    intake::{
        MediaIntake, MediaIntakeRequest, MediaLoadStart, MediaProbeDisposition, SourceIdentity,
    },
    model::MediaFile,
    probe::{MediaProbeError, probe_media},
    tools::MediaTools,
};
use std::{
    io,
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

#[derive(Default)]
pub(crate) struct SourceSession {
    intake: MediaIntake,
    inspection: Option<MediaInspection>,
    error: Option<MediaError>,
    session_id: Option<u64>,
    next_session_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceSettlement {
    Published,
    Failed,
    Unchanged,
}

impl SourceSession {
    pub(crate) fn current(&self) -> Option<&MediaFile> {
        self.intake.current()
    }
    pub(crate) fn current_with_identity(&self) -> Option<(&SourceIdentity, &MediaFile)> {
        self.intake.current_with_identity()
    }
    pub(crate) fn session_id(&self) -> Option<u64> {
        self.session_id
    }
    pub(crate) fn is_inspecting(&self) -> bool {
        self.inspection.is_some()
    }
    pub(crate) fn error(&self) -> Option<&MediaError> {
        self.error.as_ref()
    }
    pub(crate) fn set_error(&mut self, error: MediaError) {
        self.error = Some(error);
    }
    pub(crate) fn clear_error(&mut self) {
        self.error = None;
    }

    // Source and independent queue/batch editors share an identifier space so a retired
    // worker or dialog cannot accidentally target another editor.
    pub(crate) fn allocate_session_id(&mut self) -> u64 {
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .expect("Apollo session identifier space is exhausted");
        self.next_session_id
    }

    pub(crate) fn begin_load(&mut self, path: &Path) -> Option<MediaLoadStart> {
        self.clear_error();
        self.inspection = None;
        match self.intake.begin_load(path) {
            Ok(start) => Some(start),
            Err(error) => {
                self.set_error(MediaError::from_intake_error(&error));
                None
            }
        }
    }

    pub(crate) fn start_inspection(
        &mut self,
        tools: MediaTools,
        request: MediaIntakeRequest,
        wake: impl FnOnce() + Send + 'static,
    ) {
        let path = request.path().to_path_buf();
        self.start_inspection_with(request, wake, move || probe_media(&tools, &path));
    }

    fn start_inspection_with(
        &mut self,
        request: MediaIntakeRequest,
        wake: impl FnOnce() + Send + 'static,
        inspect: impl FnOnce() -> Result<MediaFile, MediaProbeError> + Send + 'static,
    ) {
        if self.inspection.is_some() || !self.intake.is_pending(&request) {
            return;
        }
        match MediaInspection::spawn_with(request.clone(), wake, inspect) {
            Ok(inspection) => self.inspection = Some(inspection),
            Err(error) => {
                self.fail_inspection(
                    &request,
                    format!("Media inspection worker could not start: {error}"),
                );
            }
        }
    }

    fn fail_inspection(
        &mut self,
        request: &MediaIntakeRequest,
        message: impl Into<String>,
    ) -> SourceSettlement {
        if self.intake.retire_probe(request) {
            self.set_error(MediaError::probe_failure(message.into()));
            SourceSettlement::Failed
        } else {
            SourceSettlement::Unchanged
        }
    }

    pub(crate) fn poll(&mut self) -> SourceSettlement {
        let Some(inspection) = self.inspection.as_ref() else {
            return SourceSettlement::Unchanged;
        };
        let result = inspection.receiver.try_recv();
        if matches!(result, Err(TryRecvError::Empty)) {
            return SourceSettlement::Unchanged;
        }
        let Some(inspection) = self.inspection.take() else {
            return SourceSettlement::Unchanged;
        };
        match result {
            Ok(result) => self.complete_probe(inspection.request, result),
            Err(TryRecvError::Disconnected) => self.fail_inspection(
                &inspection.request,
                "Media inspection worker stopped before returning a result.",
            ),
            Err(TryRecvError::Empty) => unreachable!("empty results return before settlement"),
        }
    }

    pub(crate) fn complete_probe(
        &mut self,
        request: MediaIntakeRequest,
        result: Result<MediaFile, MediaProbeError>,
    ) -> SourceSettlement {
        match self.intake.complete_probe(request, result) {
            Ok(MediaProbeDisposition::Published) => {
                self.publish();
                SourceSettlement::Published
            }
            Ok(MediaProbeDisposition::Retired) => SourceSettlement::Unchanged,
            Err(error) => {
                self.set_error(MediaError::from_intake_error(&error));
                SourceSettlement::Failed
            }
        }
    }

    fn publish(&mut self) {
        self.session_id = Some(self.allocate_session_id());
        self.clear_error();
    }

    pub(crate) fn restore(&mut self, identity: SourceIdentity, media: MediaFile) {
        self.inspection = None;
        self.intake.restore_retained(identity, media);
        self.publish();
    }

    pub(crate) fn close(&mut self) {
        self.inspection = None;
        self.intake.close_source();
        self.session_id = None;
        self.clear_error();
    }

    #[cfg(test)]
    pub(crate) fn attach_inspection(
        &mut self,
        request: MediaIntakeRequest,
        receiver: Receiver<Result<MediaFile, MediaProbeError>>,
    ) {
        self.inspection = Some(MediaInspection { request, receiver });
    }

    #[cfg(test)]
    pub(crate) fn inspection_path(&self) -> Option<&Path> {
        self.inspection
            .as_ref()
            .map(|inspection| inspection.request.path())
    }
}

impl super::Application {
    /// One admission gate for native selection and external file drops.
    #[cfg(any(windows, test))]
    pub(crate) fn can_accept_source_paths(&self) -> bool {
        !self.is_closing()
            && self.discovery.ready().is_some()
            && self.batch_origin().is_some()
            && !self.source.is_inspecting()
            && !self.frame_work.copy_active()
    }

    #[cfg(any(windows, test))]
    pub(crate) fn accept_source_paths(
        &mut self,
        origin: super::dialogs::InputDialogOrigin,
        paths: Vec<std::path::PathBuf>,
        wake: impl Fn() + Clone + Send + 'static,
    ) {
        if !origin.is_current(self) || !self.can_accept_source_paths() {
            return;
        }
        let Some(path) = paths.first() else {
            return;
        };
        let Some(tools) = self.discovery.ready().cloned() else {
            return;
        };
        self.settings.remember_source(path);
        if paths.len() > 1 {
            self.start_batch_paths(paths, wake);
            return;
        }
        self.queue_confirmation = None;
        self.operation_action_error = None;
        match self.source.begin_load(path) {
            Some(MediaLoadStart::Reused) => {
                self.enter_workspace(
                    self.active_operation()
                        .unwrap_or(super::Operation::AudioMix),
                );
            }
            Some(MediaLoadStart::Probe(request)) => {
                self.source.start_inspection(tools, request, wake)
            }
            None => {}
        }
    }
}

struct MediaInspection {
    request: MediaIntakeRequest,
    receiver: Receiver<Result<MediaFile, MediaProbeError>>,
}

impl MediaInspection {
    fn spawn_with<F>(
        request: MediaIntakeRequest,
        wake: impl FnOnce() + Send + 'static,
        inspect: F,
    ) -> io::Result<Self>
    where
        F: FnOnce() -> Result<MediaFile, MediaProbeError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-media-inspection".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(inspect));
                if let Ok(result) = result {
                    let _ = sender.send(result);
                }
                drop(sender);
                wake();
            })?;

        Ok(Self { request, receiver })
    }
}

#[cfg(test)]
mod tests;

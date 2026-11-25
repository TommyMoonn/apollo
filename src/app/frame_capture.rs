//! Renderer-independent interactive frame state and worker ownership.
use crate::frame_capture::{
    clipboard::{
        CopyFrameCompletion, CopyFrameError, CopyFrameExtractionOwner, CopyFrameRequest,
        CopyFrameWorker,
    },
    draft::FrameCaptureDraft,
    frame_index::{
        FrameDirection, FrameIndexCompletion, FrameIndexError, FrameIndexQueryOwner,
        FrameIndexRequest, FrameIndexState, FrameNavigation, FrameNavigator, FrameStep,
    },
    model::FramePosition,
    preview::{
        FramePreview, PREVIEW_DEBOUNCE, PreviewCompletion, PreviewExtractionError,
        PreviewExtractionOwner, PreviewImage, PreviewRequest, decode_preview_png,
    },
    thumbnail::{FrameCaptureThumbnail, decode_frame_capture_thumbnail},
};
use crate::media::{
    error::MediaError,
    intake::SourceIdentity,
    model::MediaFile,
    process::{MediaProcessStartError, MediaProcessWake},
    tools::MediaTools,
};
use std::{
    io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Instant,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameNavigationAvailability {
    pub previous: bool,
    pub next: bool,
    pub loading: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionSettle {
    Debounced,
    Immediate,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameCopyStatus {
    #[default]
    Idle,
    Copying,
    Copied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameCaptureEditorState {
    pub(crate) selection: FrameCaptureDraft,
    pub(crate) session_id: u64,
    pub(crate) source_identity: Option<SourceIdentity>,
    pub(crate) preview: FramePreview<PreviewImage>,
    pub(crate) frame_navigator: FrameNavigator,
    pub(crate) frame_index_due: Option<(FramePosition, Instant)>,
    pub(crate) pending_step: Option<PendingFrameStep>,
    pub(crate) copy_status: FrameCopyStatus,
}

impl FrameCaptureEditorState {
    pub(crate) fn new(
        session_id: u64,
        source_identity: Option<SourceIdentity>,
        media: &MediaFile,
    ) -> Self {
        Self {
            selection: FrameCaptureDraft::new(media),
            session_id,
            source_identity,
            preview: FramePreview::default(),
            frame_navigator: FrameNavigator::default(),
            frame_index_due: None,
            pending_step: None,
            copy_status: FrameCopyStatus::Idle,
        }
    }

    pub(crate) fn hydrate(
        &mut self,
        media: &MediaFile,
        request: &crate::frame_capture::model::FrameCaptureRequest,
    ) {
        self.selection = FrameCaptureDraft::from_request(media, request);
    }
}

impl Deref for FrameCaptureEditorState {
    type Target = FrameCaptureDraft;

    fn deref(&self) -> &Self::Target {
        &self.selection
    }
}

impl DerefMut for FrameCaptureEditorState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.selection
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingFrameStep {
    pub(crate) origin: FramePosition,
    pub(crate) direction: FrameDirection,
    pub(crate) step: FrameStep,
}

pub(crate) fn frame_navigation_availability(
    media: &MediaFile,
    editor: &FrameCaptureEditorState,
) -> FrameNavigationAvailability {
    if editor.source_identity.is_none() {
        return FrameNavigationAvailability {
            previous: false,
            next: false,
            loading: false,
            failed: false,
        };
    }
    let Some(position) = editor.selection.valid_position(media) else {
        return FrameNavigationAvailability {
            previous: false,
            next: false,
            loading: false,
            failed: false,
        };
    };
    let loading = editor.frame_index_due.is_some()
        || matches!(
            editor.frame_navigator.state(),
            FrameIndexState::Loading { .. }
        );
    let failed = matches!(
        editor.frame_navigator.state(),
        FrameIndexState::Failed { .. }
    );
    let step = editor.selection.step();
    let previous = !matches!(
        editor
            .frame_navigator
            .navigate(position, FrameDirection::Previous, step),
        FrameNavigation::Boundary
    );
    let next = !matches!(
        editor
            .frame_navigator
            .navigate(position, FrameDirection::Next, step),
        FrameNavigation::Boundary
    );
    FrameNavigationAvailability {
        previous,
        next,
        loading,
        failed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCaptureThumbnailRequest {
    request_id: u64,
    output: PathBuf,
}

impl FrameCaptureThumbnailRequest {
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn output(&self) -> &Path {
        &self.output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FrameCaptureThumbnailState {
    #[default]
    Empty,
    Loading {
        request_id: u64,
    },
    Ready {
        request_id: u64,
        preview: FrameCaptureThumbnail,
    },
    Failed {
        request_id: u64,
        diagnostic: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CaptureThumbnail {
    next_thumbnail_request_id: u64,
    thumbnail: FrameCaptureThumbnailState,
}
impl CaptureThumbnail {
    pub fn thumbnail(&self) -> &FrameCaptureThumbnailState {
        &self.thumbnail
    }

    pub fn settle_thumbnail(
        &mut self,
        request_id: u64,
        result: Result<FrameCaptureThumbnail, String>,
    ) -> bool {
        if !matches!(
            self.thumbnail,
            FrameCaptureThumbnailState::Loading {
                request_id: active_request_id
            } if active_request_id == request_id
        ) {
            return false;
        }

        self.thumbnail = match result {
            Ok(preview) => FrameCaptureThumbnailState::Ready {
                request_id,
                preview,
            },
            Err(diagnostic) => FrameCaptureThumbnailState::Failed {
                request_id,
                diagnostic,
            },
        };
        true
    }

    pub fn begin_thumbnail(&mut self, output: &Path) -> Option<FrameCaptureThumbnailRequest> {
        if !matches!(self.thumbnail, FrameCaptureThumbnailState::Empty) {
            return None;
        }

        self.next_thumbnail_request_id = self
            .next_thumbnail_request_id
            .checked_add(1)
            .expect("frame capture thumbnail request identifier space is exhausted");
        let request_id = self.next_thumbnail_request_id;
        self.thumbnail = FrameCaptureThumbnailState::Loading { request_id };
        Some(FrameCaptureThumbnailRequest {
            request_id,
            output: output.to_path_buf(),
        })
    }

    pub(crate) fn reset(&mut self) {
        self.thumbnail = FrameCaptureThumbnailState::Empty;
    }
}
pub(crate) struct FrameCaptureThumbnailLoad {
    pub(crate) session_id: u64,
    pub(crate) request_id: u64,
    pub(crate) receiver: Receiver<Result<FrameCaptureThumbnail, String>>,
}

pub(crate) struct FramePreviewDecode {
    pub(crate) session_id: u64,
    pub(crate) request: PreviewRequest,
    pub(crate) receiver: Receiver<Result<PreviewImage, PreviewExtractionError>>,
}

impl FrameCaptureThumbnailLoad {
    pub(crate) fn spawn(
        session_id: u64,
        request: &FrameCaptureThumbnailRequest,
        wake: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        let output = request.output().to_path_buf();
        Self::spawn_with(session_id, request.request_id(), wake, move || {
            decode_frame_capture_thumbnail(&output)
        })
    }

    pub(crate) fn spawn_with<F>(
        session_id: u64,
        request_id: u64,
        wake: impl FnOnce() + Send + 'static,
        decode: F,
    ) -> io::Result<Self>
    where
        F: FnOnce() -> Result<FrameCaptureThumbnail, String> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-frame-thumbnail".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode));
                if let Ok(result) = result {
                    let _ = sender.send(result);
                }
                drop(sender);
                wake();
            })?;
        Ok(Self {
            session_id,
            request_id,
            receiver,
        })
    }
}

impl FramePreviewDecode {
    pub(crate) fn spawn(
        session_id: u64,
        request: PreviewRequest,
        bytes: Vec<u8>,
        wake: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with(session_id, request, wake, move || {
            decode_preview_png(&bytes)
        })
    }

    pub(crate) fn spawn_with<F>(
        session_id: u64,
        request: PreviewRequest,
        wake: impl FnOnce() + Send + 'static,
        decode: F,
    ) -> io::Result<Self>
    where
        F: FnOnce() -> Result<PreviewImage, PreviewExtractionError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-frame-preview-decode".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode))
                    .unwrap_or_else(|_| {
                        Err(PreviewExtractionError::Decode(
                            "preview decode worker stopped unexpectedly".to_owned(),
                        ))
                    });
                let _ = sender.send(result);
                wake();
            })?;
        Ok(Self {
            session_id,
            request,
            receiver,
        })
    }
}

#[derive(Default)]
pub(crate) struct FrameCaptureWork {
    pub(crate) frame_capture_thumbnail_load: Option<FrameCaptureThumbnailLoad>,
    pub(crate) frame_preview_extraction: PreviewExtractionOwner,
    pub(crate) frame_preview_decode: Option<FramePreviewDecode>,
    pub(crate) frame_index_query: FrameIndexQueryOwner,
    pub(crate) copy_frame_extraction: CopyFrameExtractionOwner,
    pub(crate) copy_frame_worker: Option<CopyFrameWorker>,
    copy_frame_session_id: Option<u64>,
    preview_session: Option<u64>,
    index_session: Option<u64>,
}

impl FrameCaptureEditorState {
    pub(crate) fn retire_index(&mut self) {
        self.pending_step = None;
        self.frame_navigator.invalidate();
        self.frame_index_due = None;
    }

    // Temporary navigation away retains already accepted pixels and neighborhoods.
    pub(crate) fn suspend_support(&mut self) {
        if matches!(
            self.preview.state(),
            crate::frame_capture::preview::PreviewState::Debouncing { .. }
                | crate::frame_capture::preview::PreviewState::Loading { .. }
        ) {
            self.preview.invalidate();
        }
        if matches!(
            self.frame_navigator.state(),
            FrameIndexState::Loading { .. }
        ) {
            self.frame_navigator.invalidate();
        }
        self.frame_index_due = None;
        self.pending_step = None;
    }

    pub(crate) fn retire_support(&mut self) {
        self.preview.invalidate();
        self.retire_index();
    }

    pub(crate) fn schedule_preview(
        &mut self,
        media: &MediaFile,
        position: Option<FramePosition>,
        settle: PositionSettle,
        now: Instant,
    ) -> Option<Instant> {
        let source = self.source_identity.as_ref()?;
        let Some(position) = position else {
            self.preview.invalidate();
            return None;
        };
        match settle {
            PositionSettle::Debounced => self
                .preview
                .schedule_position(source, media, position, now)
                .map(|_| now + PREVIEW_DEBOUNCE),
            PositionSettle::Immediate => self
                .preview
                .schedule_position_immediately(source, media, position, now)
                .map(|_| now),
        }
    }

    pub(crate) fn accepts_preview(&self, session: u64, request: &PreviewRequest) -> bool {
        self.session_id == session
            && self.source_identity.as_ref() == Some(&request.source_identity)
            && self.preview.is_loading(request)
    }

    pub(crate) fn settle_preview(
        &mut self,
        session: u64,
        request: PreviewRequest,
        result: Result<PreviewImage, PreviewExtractionError>,
    ) -> bool {
        self.accepts_preview(session, &request) && self.preview.settle(request, result)
    }

    pub(crate) fn take_due_index(&mut self, now: Instant) -> Option<FramePosition> {
        self.frame_index_due
            .filter(|(_, deadline)| now >= *deadline)?;
        self.frame_index_due.take().map(|(position, _)| position)
    }

    pub(crate) fn index_loading_at(&self, position: FramePosition) -> bool {
        matches!(self.frame_navigator.state(), FrameIndexState::Loading { request } if request.center == position && self.source_identity.as_ref() == Some(&request.source_identity))
    }

    pub(crate) fn schedule_index(
        &mut self,
        media: &MediaFile,
        position: FramePosition,
    ) -> Result<Option<FrameIndexRequest>, FrameIndexError> {
        let Some(source) = self.source_identity.as_ref() else {
            return Err(FrameIndexError::InvalidPosition);
        };
        self.frame_index_due = None;
        self.frame_navigator.schedule(source, media, position)
    }

    pub(crate) fn settle_index(
        &mut self,
        session: u64,
        request: FrameIndexRequest,
        result: Result<Vec<u8>, FrameIndexError>,
    ) -> bool {
        if self.session_id != session
            || self.source_identity.as_ref() != Some(&request.source_identity)
            || !self.frame_navigator.settle(request, result)
        {
            return false;
        }
        if matches!(self.frame_navigator.state(), FrameIndexState::Ready { .. }) {
            true
        } else {
            self.pending_step = None;
            false
        }
    }

    pub(crate) fn request_step(
        &mut self,
        media: &MediaFile,
        direction: FrameDirection,
    ) -> Option<FrameNavigation> {
        self.source_identity.as_ref()?;
        let origin = self.selection.valid_position(media)?;
        self.pending_step = Some(PendingFrameStep {
            origin,
            direction,
            step: self.selection.step(),
        });
        self.pending_outcome()
    }

    pub(crate) fn pending_outcome(&mut self) -> Option<FrameNavigation> {
        let pending = self.pending_step?;
        let outcome =
            self.frame_navigator
                .navigate(pending.origin, pending.direction, pending.step);
        if matches!(outcome, FrameNavigation::Boundary) {
            self.pending_step = None;
        }
        Some(outcome)
    }

    pub(crate) fn apply_resolved_step(
        &mut self,
        media: &MediaFile,
        position: FramePosition,
    ) -> bool {
        let Some(pending) = self.pending_step.take() else {
            return false;
        };
        if self.selection.valid_position(media) != Some(pending.origin) {
            return false;
        }
        self.frame_index_due = None;
        self.selection.set_position(media, position);
        true
    }
}

impl FrameCaptureWork {
    pub(crate) fn copy_active(&self) -> bool {
        self.copy_frame_extraction.is_active() || self.copy_frame_worker.is_some()
    }

    pub(crate) fn cancel_preview(&mut self) {
        self.frame_preview_extraction.cancel();
        self.preview_session = None;
        self.frame_preview_decode = None;
    }

    pub(crate) fn cancel_index(&mut self) -> bool {
        self.index_session = None;
        self.frame_index_query.cancel()
    }

    pub(crate) fn start_preview(
        &mut self,
        session: u64,
        tools: &MediaTools,
        request: PreviewRequest,
        wake: MediaProcessWake,
    ) -> Result<(), MediaProcessStartError> {
        self.cancel_preview();
        self.frame_preview_extraction.start(tools, request, wake)?;
        self.preview_session = Some(session);
        Ok(())
    }

    pub(crate) fn poll_preview(&mut self) -> Option<(u64, PreviewCompletion)> {
        let completion = self.frame_preview_extraction.poll()?;
        Some((self.preview_session.take()?, completion))
    }

    pub(crate) fn index_owned_by(&self, session: u64) -> bool {
        self.index_session == Some(session)
    }

    pub(crate) fn start_index(
        &mut self,
        session: u64,
        tools: &MediaTools,
        request: FrameIndexRequest,
        wake: MediaProcessWake,
    ) -> Result<(), FrameIndexError> {
        self.cancel_index();
        self.frame_index_query.start(tools, request, wake)?;
        self.index_session = Some(session);
        Ok(())
    }

    pub(crate) fn poll_index(&mut self) -> Option<(u64, FrameIndexCompletion)> {
        let completion = self.frame_index_query.poll()?;
        Some((self.index_session.take()?, completion))
    }

    pub(crate) fn start_decode(
        &mut self,
        session: u64,
        request: PreviewRequest,
        bytes: Vec<u8>,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<(), PreviewExtractionError> {
        self.frame_preview_decode = Some(
            FramePreviewDecode::spawn(session, request, bytes, wake).map_err(|error| {
                PreviewExtractionError::Decode(format!(
                    "preview decode worker could not start: {error}"
                ))
            })?,
        );
        Ok(())
    }

    pub(crate) fn poll_decode(
        &mut self,
    ) -> Option<(
        u64,
        PreviewRequest,
        Result<PreviewImage, PreviewExtractionError>,
    )> {
        let result = match self.frame_preview_decode.as_ref()?.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(PreviewExtractionError::Decode(
                "preview decode worker stopped before returning a result".to_owned(),
            )),
        };
        let decode = self.frame_preview_decode.take()?;
        Some((decode.session_id, decode.request, result))
    }
}

impl FrameCaptureEditorState {
    pub(crate) fn copy_request(
        &self,
        media: &MediaFile,
    ) -> Option<Result<CopyFrameRequest, CopyFrameError>> {
        let source = self.source_identity.as_ref()?;
        let position = self.selection.valid_position(media)?;
        Some(CopyFrameRequest::new(source, media, position))
    }

    pub(crate) fn accepts_copy(&self, session: u64, request: Option<&CopyFrameRequest>) -> bool {
        self.session_id == session
            && request.is_none_or(|request| {
                self.source_identity.as_ref() == Some(&request.source_identity)
            })
    }

    pub(crate) fn settle_copy(&mut self, result: Result<(), CopyFrameError>) -> Option<MediaError> {
        match result {
            Ok(()) => {
                self.copy_status = FrameCopyStatus::Copied;
                None
            }
            Err(CopyFrameError::Cancelled) => {
                self.copy_status = FrameCopyStatus::Idle;
                None
            }
            Err(error) => {
                self.copy_status = FrameCopyStatus::Idle;
                Some(error.user_error())
            }
        }
    }
}

impl FrameCaptureWork {
    pub(crate) fn start_copy(
        &mut self,
        session: u64,
        tools: &MediaTools,
        request: CopyFrameRequest,
        wake: MediaProcessWake,
    ) -> Result<(), CopyFrameError> {
        self.copy_frame_extraction.start(tools, request, wake)?;
        self.copy_frame_session_id = Some(session);
        Ok(())
    }

    pub(crate) fn poll_copy_extraction(&mut self) -> Option<(u64, CopyFrameCompletion)> {
        let completion = self.copy_frame_extraction.poll()?;
        Some((self.copy_frame_session_id.take()?, completion))
    }

    pub(crate) fn start_copy_publication(
        &mut self,
        session: u64,
        request: CopyFrameRequest,
        bytes: Vec<u8>,
        native_window: Option<isize>,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<(), CopyFrameError> {
        self.copy_frame_worker = Some(CopyFrameWorker::spawn(request, bytes, native_window, wake)?);
        self.copy_frame_session_id = Some(session);
        Ok(())
    }

    pub(crate) fn poll_copy_publication(
        &mut self,
    ) -> Option<(u64, CopyFrameRequest, Result<(), CopyFrameError>)> {
        let result = self.copy_frame_worker.as_ref()?.poll()?;
        let worker = self.copy_frame_worker.take()?;
        Some((
            self.copy_frame_session_id.take()?,
            worker.request.clone(),
            result,
        ))
    }

    pub(crate) fn cancel_copy(&mut self) {
        self.copy_frame_extraction.cancel();
        self.copy_frame_worker = None;
        self.copy_frame_session_id = None;
    }

    pub(crate) fn start_thumbnail(
        &mut self,
        session: u64,
        request: &FrameCaptureThumbnailRequest,
        wake: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
        self.frame_capture_thumbnail_load =
            Some(FrameCaptureThumbnailLoad::spawn(session, request, wake)?);
        Ok(())
    }

    pub(crate) fn poll_thumbnail(
        &mut self,
    ) -> Option<(u64, u64, Result<FrameCaptureThumbnail, String>)> {
        let result = match self
            .frame_capture_thumbnail_load
            .as_ref()?
            .receiver
            .try_recv()
        {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                Err("Frame Capture preview worker stopped before returning a result.".to_owned())
            }
        };
        let load = self.frame_capture_thumbnail_load.take()?;
        Some((load.session_id, load.request_id, result))
    }

    pub(crate) fn cancel_thumbnail(&mut self) {
        self.frame_capture_thumbnail_load = None;
    }
}

#[cfg(test)]
mod tests;

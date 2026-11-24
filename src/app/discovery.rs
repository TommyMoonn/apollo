//! Application ownership of progressive media-tool discovery.

use std::{
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use crate::media::tools::{
    MediaTools, MediaToolsError, VideoEncoderCapabilities, VideoEncoderCapability,
    cached_video_encoder_capabilities, discover_base_media_tools,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaToolsState {
    Checking,
    Ready(MediaTools),
    Unavailable(MediaToolsError),
}

struct DiscoveryAttempt {
    id: u64,
    base: Option<Receiver<MediaTools>>,
    completion: Receiver<Result<MediaTools, MediaToolsError>>,
}

pub(crate) struct MediaToolsDiscovery {
    state: MediaToolsState,
    pending: Option<DiscoveryAttempt>,
    next_attempt_id: u64,
}

impl Default for MediaToolsDiscovery {
    fn default() -> Self {
        Self {
            state: MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(
                "Media tool discovery has not started.".to_owned(),
            )),
            pending: None,
            next_attempt_id: 0,
        }
    }
}

impl MediaToolsDiscovery {
    pub(crate) fn state(&self) -> &MediaToolsState {
        &self.state
    }

    pub(crate) fn ready(&self) -> Option<&MediaTools> {
        match &self.state {
            MediaToolsState::Ready(tools) => Some(tools),
            _ => None,
        }
    }

    pub(crate) fn settled_capabilities(&self) -> Option<&VideoEncoderCapabilities> {
        if self.is_pending() {
            return None;
        }
        self.ready().map(|tools| &tools.video_encoders)
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn start(&mut self, wake: impl Fn() + Send + 'static) -> bool {
        self.start_with(
            wake,
            discover_base_media_tools,
            cached_video_encoder_capabilities,
        )
    }

    pub(crate) fn start_with<B, C>(
        &mut self,
        wake: impl Fn() + Send + 'static,
        base: B,
        capabilities: C,
    ) -> bool
    where
        B: FnOnce() -> Result<MediaTools, MediaToolsError> + Send + 'static,
        C: FnOnce(&Path) -> VideoEncoderCapabilities + Send + 'static,
    {
        if self.is_pending() {
            return false;
        }
        let id = self.next_attempt_id;
        self.next_attempt_id = id
            .checked_add(1)
            .expect("Discovery attempt identifier space is exhausted");
        let (base_sender, base_receiver) = mpsc::sync_channel(1);
        let (completion_sender, completion) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("apollo-media-tools".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut tools = base()?;
                    if base_sender.send(tools.clone()).is_err() {
                        return Ok(tools);
                    }
                    wake();
                    tools.video_encoders =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            capabilities(&tools.ffmpeg)
                        }))
                        .unwrap_or_else(|_| {
                            unavailable_capabilities(
                                "The encoder capability worker stopped unexpectedly.".to_owned(),
                            )
                        });
                    Ok(tools)
                }))
                .unwrap_or_else(|_| {
                    Err(MediaToolsError::DiscoveryFailed(
                        "The media tool discovery worker stopped unexpectedly.".to_owned(),
                    ))
                });
                let _ = completion_sender.send(result);
                wake();
            });
        match worker {
            Ok(_) => {
                self.state = MediaToolsState::Checking;
                self.pending = Some(DiscoveryAttempt {
                    id,
                    base: Some(base_receiver),
                    completion,
                });
                true
            }
            Err(error) => {
                self.state = MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(
                    format!("Media tool discovery worker could not start: {error}"),
                ));
                false
            }
        }
    }

    fn accept_base(&mut self) {
        let Some(attempt) = self.pending.as_mut() else {
            return;
        };
        let Some(receiver) = &attempt.base else {
            return;
        };
        if let Ok(tools) = receiver.try_recv() {
            if matches!(self.state, MediaToolsState::Checking) {
                self.state = MediaToolsState::Ready(tools);
            }
            attempt.base = None;
        }
    }

    /// Returns true when the current attempt reaches a terminal state.
    pub(crate) fn poll(&mut self) -> bool {
        self.accept_base();
        let Some(attempt) = &self.pending else {
            return false;
        };
        let id = attempt.id;
        let result = match attempt.completion.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err(MediaToolsError::DiscoveryFailed(
                "The media tool discovery worker stopped before returning a result.".to_owned(),
            )),
        };
        // The worker may publish base tools between the first read and terminal read.
        self.accept_base();
        if self.complete(id, result) {
            return true;
        }
        // A mismatched terminal result must not replace the accepted pair or leave
        // the capability check pending indefinitely.
        self.complete(
            id,
            Err(MediaToolsError::DiscoveryFailed(
                "The media tool discovery result did not match the active tool pair.".to_owned(),
            )),
        )
    }

    fn complete(&mut self, id: u64, result: Result<MediaTools, MediaToolsError>) -> bool {
        if !self
            .pending
            .as_ref()
            .is_some_and(|attempt| attempt.id == id)
        {
            return false;
        }
        match (&mut self.state, result) {
            (MediaToolsState::Checking, result) => {
                self.state = match result {
                    Ok(tools) => MediaToolsState::Ready(tools),
                    Err(error) => MediaToolsState::Unavailable(error),
                };
            }
            (MediaToolsState::Ready(current), Ok(completed))
                if current.ffmpeg == completed.ffmpeg && current.ffprobe == completed.ffprobe =>
            {
                current.video_encoders = completed.video_encoders;
            }
            (MediaToolsState::Ready(current), Err(error)) => {
                current.video_encoders = unavailable_capabilities(error.to_string());
            }
            _ => return false,
        }
        self.pending = None;
        true
    }

    #[cfg(test)]
    pub(crate) fn with_state(state: MediaToolsState) -> Self {
        Self {
            state,
            ..Self::default()
        }
    }
}

fn unavailable_capabilities(reason: String) -> VideoEncoderCapabilities {
    let failure = VideoEncoderCapability::RuntimeUnavailable { reason };
    VideoEncoderCapabilities {
        h264_nvenc: failure.clone(),
        h265_nvenc: failure,
    }
}

#[cfg(test)]
mod tests;

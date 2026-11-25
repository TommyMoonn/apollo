//! Dialog routing identity and request construction, independent of native UI.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::{Application, Operation};
use crate::{
    audio_extract::{draft::AudioExtractDraft, model::AudioExtractFormat},
    audio_mix::{draft::AudioMixDraft, model::OutputContainer},
    frame_capture::{draft::FrameCaptureDraft, model::FrameCaptureFormat},
    media::container::MediaContainer,
    remux::draft::RemuxDraft,
    upscale::draft::UpscaleDraft,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogTarget {
    MediaInput,
    BatchInput,
    BatchAppend,
    AudioMixOutput { session_id: u64 },
    UpscaleOutput { session_id: u64 },
    RemuxOutput { session_id: u64 },
    AudioExtractOutput { session_id: u64 },
    FrameCaptureOutput { session_id: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Mp4,
    Mkv,
    Mka,
    M4a,
    Flac,
    Png,
    Jpeg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SaveDialogOptions {
    pub(crate) initial_directory: Option<PathBuf>,
    pub(crate) file_name: OsString,
    pub(crate) format: OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileDialogRequest {
    OpenSource {
        initial_directory: Option<PathBuf>,
    },
    OpenSources {
        initial_directory: Option<PathBuf>,
        target: DialogTarget,
    },
    SaveOutput {
        target: DialogTarget,
        options: SaveDialogOptions,
    },
}

impl FileDialogRequest {
    pub(crate) fn target(&self) -> DialogTarget {
        match self {
            Self::OpenSource { .. } => DialogTarget::MediaInput,
            Self::OpenSources { target, .. } => *target,
            Self::SaveOutput { target, .. } => *target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DialogOutcome {
    Selected(PathBuf),
    SelectedMany(Vec<PathBuf>),
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileDialogCompletion {
    pub(crate) target: DialogTarget,
    pub(crate) outcome: DialogOutcome,
}

/// Input dialogs must not replace a source or append to a different batch after navigation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputDialogOrigin {
    navigation_revision: u64,
    source_session: Option<u64>,
}

impl InputDialogOrigin {
    pub(crate) fn capture(app: &Application) -> Self {
        Self {
            navigation_revision: app.navigation_revision(),
            source_session: app.source.session_id(),
        }
    }

    pub(crate) fn is_current(self, app: &Application) -> bool {
        self == Self::capture(app)
    }
}

impl DialogTarget {
    pub(crate) fn output(self) -> Option<(Operation, u64)> {
        Some(match self {
            Self::AudioMixOutput { session_id } => (Operation::AudioMix, session_id),
            Self::UpscaleOutput { session_id } => (Operation::VideoUpscale, session_id),
            Self::RemuxOutput { session_id } => (Operation::Remux, session_id),
            Self::AudioExtractOutput { session_id } => (Operation::AudioExtract, session_id),
            Self::FrameCaptureOutput { session_id } => (Operation::FrameCapture, session_id),
            Self::MediaInput | Self::BatchInput | Self::BatchAppend => return None,
        })
    }
}

impl Application {
    pub(crate) fn prepare_file_dialog_request(
        &self,
        mut request: FileDialogRequest,
    ) -> FileDialogRequest {
        match &mut request {
            FileDialogRequest::OpenSource { initial_directory }
            | FileDialogRequest::OpenSources {
                initial_directory, ..
            } => {
                *initial_directory = self.settings.source_directory();
            }
            FileDialogRequest::SaveOutput { options, .. } => {
                let fallback = options
                    .initial_directory
                    .as_deref()
                    .unwrap_or(Path::new("."));
                options.initial_directory = self.settings.output_directory(fallback);
            }
        }
        request
    }
}

/// The same draft supplies the path and filter in normal, queue and batch editors.
pub(crate) enum OutputDraft<'a> {
    AudioMix(&'a AudioMixDraft),
    Upscale(&'a UpscaleDraft),
    Remux(&'a RemuxDraft),
    AudioExtract(&'a AudioExtractDraft),
    FrameCapture(&'a FrameCaptureDraft),
}

impl OutputDraft<'_> {
    pub(crate) fn request(self, session_id: u64) -> FileDialogRequest {
        let (target, output, format): (DialogTarget, PathBuf, OutputFormat) = match self {
            Self::AudioMix(draft) => (
                DialogTarget::AudioMixOutput { session_id },
                draft.output_path(),
                match draft.container() {
                    OutputContainer::Mp4 => OutputFormat::Mp4,
                    OutputContainer::Mkv => OutputFormat::Mkv,
                },
            ),
            Self::Upscale(draft) => (
                DialogTarget::UpscaleOutput { session_id },
                draft.output_path(),
                OutputFormat::Mp4,
            ),
            Self::Remux(draft) => (
                DialogTarget::RemuxOutput { session_id },
                draft.output_path(),
                match draft.container() {
                    MediaContainer::Mp4 => OutputFormat::Mp4,
                    MediaContainer::Matroska => OutputFormat::Mkv,
                },
            ),
            Self::AudioExtract(draft) => (
                DialogTarget::AudioExtractOutput { session_id },
                draft.output_path(),
                match draft.format() {
                    AudioExtractFormat::Original => OutputFormat::Mka,
                    AudioExtractFormat::Aac => OutputFormat::M4a,
                    AudioExtractFormat::Flac => OutputFormat::Flac,
                },
            ),
            Self::FrameCapture(draft) => (
                DialogTarget::FrameCaptureOutput { session_id },
                draft.output_path(),
                match draft.format() {
                    FrameCaptureFormat::Png => OutputFormat::Png,
                    FrameCaptureFormat::Jpeg => OutputFormat::Jpeg,
                },
            ),
        };
        let initial_directory = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        FileDialogRequest::SaveOutput {
            target,
            options: SaveDialogOptions {
                initial_directory: Some(initial_directory),
                file_name: output.file_name().unwrap_or_default().to_os_string(),
                format,
            },
        }
    }
}

#[cfg(test)]
mod tests;

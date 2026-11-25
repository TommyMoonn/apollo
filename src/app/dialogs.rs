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

#[cfg_attr(
    not(any(windows, test)),
    allow(dead_code, reason = "Native selections are produced only on Windows")
)]
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
                *initial_directory = self
                    .settings
                    .source_directory()
                    .or_else(|| initial_directory.take().filter(|path| path.is_dir()))
                    .or_else(|| {
                        self.source
                            .current()
                            .and_then(|media| media.path.parent())
                            .filter(|path| path.is_dir())
                            .map(Path::to_path_buf)
                    });
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

impl Application {
    /// A native selection is applied only to its still-current input context.
    #[cfg(any(windows, test))]
    pub(crate) fn settle_native_dialog(
        &mut self,
        origin: InputDialogOrigin,
        completion: FileDialogCompletion,
        wake: impl Fn() + Clone + Send + 'static,
    ) {
        use crate::{
            app::NavigationState,
            media::{error::MediaError, intake::MediaLoadStart},
        };
        if self.is_closing() {
            return;
        }
        let FileDialogCompletion { target, outcome } = completion;
        if let Some((operation, session_id)) = target.output() {
            if self.settle_independent_output(operation, session_id, &outcome)
                && let DialogOutcome::Selected(path) = outcome
            {
                self.settings.remember_output(&path);
            }
            return;
        }
        if !origin.is_current(self) {
            return;
        }
        if target == DialogTarget::BatchAppend
            && (self.navigation() != NavigationState::BatchSetup || self.batch_setup.is_none())
        {
            return;
        }
        let paths = match outcome {
            DialogOutcome::Selected(path) => vec![path],
            DialogOutcome::SelectedMany(paths) => paths,
            DialogOutcome::Cancelled => return,
            DialogOutcome::Failed(diagnostic) => {
                let error = MediaError::file_picker_unavailable(diagnostic);
                if target == DialogTarget::BatchAppend {
                    self.set_batch_append_error(error);
                } else {
                    self.source.set_error(error);
                }
                return;
            }
        };
        let Some(path) = paths.first() else {
            return;
        };
        if target != DialogTarget::BatchAppend
            && (self.source.is_inspecting()
                || self.frame_work.copy_active()
                || self.batch_origin().is_none())
        {
            return;
        }
        self.settings.remember_source(path);
        if target == DialogTarget::BatchAppend {
            self.append_batch_paths(paths, wake);
            return;
        }
        if paths.len() > 1 {
            self.start_batch_paths(paths, wake);
            return;
        }
        let Some(tools) = self.discovery.ready().cloned() else {
            return;
        };
        self.queue_confirmation = None;
        self.operation_action_error = None;
        match self.source.begin_load(path) {
            Some(MediaLoadStart::Reused) => {
                self.enter_workspace(self.active_operation().unwrap_or(Operation::AudioMix))
            }
            Some(MediaLoadStart::Probe(request)) => {
                self.source.start_inspection(tools, request, wake)
            }
            None => {}
        }
    }

    /// Shared output settlement for fixed-source queue and batch editors.
    pub(crate) fn settle_independent_output(
        &mut self,
        operation: Operation,
        session_id: u64,
        outcome: &DialogOutcome,
    ) -> bool {
        use super::NavigationState;
        let editor = match self.navigation() {
            NavigationState::QueueEdit { .. } => self.queue_edit_mut().map(|edit| &mut edit.editor),
            NavigationState::BatchEdit => self.batch_edit.as_mut().map(|edit| &mut edit.editor),
            _ => None,
        };
        let Some(editor) = editor.filter(|edit| edit.session_id == session_id) else {
            return false;
        };
        settle_editor_output(editor, operation, outcome)
    }
}

fn settle_editor_output(
    edit: &mut super::editor::OperationEditorState,
    operation: Operation,
    outcome: &DialogOutcome,
) -> bool {
    match outcome {
        DialogOutcome::Selected(path) => {
            match operation {
                Operation::AudioMix => edit.audio_mix.set_custom_output_path(path.clone()),
                Operation::VideoUpscale => edit.upscale.set_custom_output_path(path.clone()),
                Operation::Remux => edit.remux.set_custom_output_path(path.clone()),
                Operation::AudioExtract => edit.audio_extract.set_custom_output_path(path.clone()),
                Operation::FrameCapture => edit.frame_capture.set_custom_output_path(path.clone()),
            }
            if edit.active_operation == operation
                && edit.error.as_ref().is_some_and(|error| {
                    error.kind() == crate::media::error::MediaErrorKind::OutputPickerUnavailable
                })
            {
                edit.error = None;
                edit.error_details_open = false;
            }
            true
        }
        DialogOutcome::Failed(diagnostic) if edit.active_operation == operation => {
            edit.error = Some(crate::media::error::MediaError::output_picker_unavailable(
                diagnostic.clone(),
            ));
            edit.error_details_open = false;
            false
        }
        _ => false,
    }
}

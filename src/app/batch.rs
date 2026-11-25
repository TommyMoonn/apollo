//! Batch configuration, inspection and atomic queue admission.
use super::{Application, BatchOrigin, NavigationState, Operation, editor::OperationEditorState};
use crate::{
    audio_extract::model::AudioExtractFormat,
    audio_mix::model::{LoudnessNormalization, OutputContainer},
    frame_capture::model::{FrameCaptureFormat, FrameTimestamp},
    media::{
        batch::BatchInspectionOwner, container::MediaContainer, error::MediaError,
        intake::SourceIdentity, model::MediaFile,
    },
    queue::{BatchEnqueueError, QueuedOperation},
    upscale::{
        draft::UpscaleEncoderCapabilities,
        model::{UpscaleEncoderSettings, UpscaleTarget, VideoCodec},
    },
};
use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BatchItemId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchReadiness {
    Inspecting,
    Ready,
    NeedsAttention,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchAvailability {
    pub can_configure: bool,
    pub can_add_files: bool,
    pub can_enqueue: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatchAction {
    SelectOperation(Operation),
    ConfigureDefaults,
    OpenQueue,
    AddFiles,
    EditItem(BatchItemId),
    ToggleIncluded(BatchItemId),
    Enqueue,
    Back,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BatchSourceState {
    Inspecting,
    Failed,
    Ready {
        source_identity: SourceIdentity,
        media: MediaFile,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BatchDefaults {
    AudioMix {
        container: OutputContainer,
        normalization: LoudnessNormalization,
    },
    Upscale {
        target: UpscaleTarget,
        codec: VideoCodec,
        encoder_settings: UpscaleEncoderSettings,
    },
    Remux {
        container: MediaContainer,
    },
    AudioExtract {
        format: AudioExtractFormat,
    },
    FrameCapture {
        timestamp: FrameTimestamp,
        format: FrameCaptureFormat,
    },
}

impl BatchDefaults {
    pub(crate) const fn operation(&self) -> Operation {
        match self {
            Self::AudioMix { .. } => Operation::AudioMix,
            Self::Upscale { .. } => Operation::VideoUpscale,
            Self::Remux { .. } => Operation::Remux,
            Self::AudioExtract { .. } => Operation::AudioExtract,
            Self::FrameCapture { .. } => Operation::FrameCapture,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatchItem {
    pub(crate) id: BatchItemId,
    pub(crate) path: PathBuf,
    pub(crate) included: bool,
    pub(crate) source: BatchSourceState,
    pub(crate) operation: Option<QueuedOperation>,
    pub(crate) readiness: BatchReadiness,
    pub(crate) reason: Option<String>,
    pub(crate) overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatchSetupState {
    pub(crate) origin: BatchOrigin,
    pub(crate) operation: Operation,
    pub(crate) defaults: Option<BatchDefaults>,
    pub(crate) items: Vec<BatchItem>,
    pub(crate) error: Option<String>,
    pub(crate) append_error: Option<MediaError>,
}

pub(crate) struct BatchInspectionSession {
    owner: BatchInspectionOwner,
    item_ids: Vec<BatchItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BatchEditTarget {
    Defaults,
    Item(BatchItemId),
}

pub(crate) struct BatchEditState {
    pub(crate) target: BatchEditTarget,
    pub(crate) editor: OperationEditorState,
}

impl Deref for BatchEditState {
    type Target = OperationEditorState;

    fn deref(&self) -> &Self::Target {
        &self.editor
    }
}

impl DerefMut for BatchEditState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.editor
    }
}

impl BatchItem {
    pub(crate) fn can_toggle_included(&self) -> bool {
        matches!(self.source, BatchSourceState::Ready { .. })
    }

    pub(crate) fn can_edit(&self) -> bool {
        self.can_toggle_included()
            && matches!(
                self.readiness,
                BatchReadiness::Ready | BatchReadiness::NeedsAttention
            )
    }

    fn fail_inspection(&mut self, reason: String) {
        self.source = BatchSourceState::Failed;
        self.included = false;
        self.operation = None;
        self.readiness = BatchReadiness::Incompatible;
        self.reason = Some(reason);
    }
}

impl BatchSetupState {
    pub(crate) fn can_configure(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item.source, BatchSourceState::Ready { .. }))
    }

    pub(crate) fn can_enqueue(&self) -> bool {
        let mut included = self.items.iter().filter(|item| item.included).peekable();
        included.peek().is_some()
            && included.all(|item| {
                item.can_toggle_included()
                    && item.readiness == BatchReadiness::Ready
                    && item.operation.is_some()
            })
    }
}

impl Application {
    pub(crate) fn start_batch_paths(
        &mut self,
        paths: Vec<PathBuf>,
        wake: impl Fn() + Clone + Send + 'static,
    ) -> bool {
        let Some(tools) = self.discovery.ready().cloned() else {
            return false;
        };
        self.start_batch_paths_with(paths, |paths| {
            BatchInspectionOwner::spawn(tools, paths, wake)
        })
    }

    fn start_batch_paths_with(
        &mut self,
        paths: Vec<PathBuf>,
        spawn: impl FnOnce(Vec<PathBuf>) -> std::io::Result<BatchInspectionOwner>,
    ) -> bool {
        if paths.len() < 2 || self.source.is_inspecting() || self.frame_work.copy_active() {
            return false;
        }
        let Some(origin) = self.batch_origin() else {
            return false;
        };
        let items = paths
            .iter()
            .enumerate()
            .map(|(index, path)| BatchItem {
                id: BatchItemId(index),
                path: path.clone(),
                included: true,
                source: BatchSourceState::Inspecting,
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            })
            .collect::<Vec<_>>();
        let item_ids = items.iter().map(|item| item.id).collect();
        let inspection = match spawn(paths) {
            Ok(inspection) => inspection,
            Err(error) => {
                self.source.set_error(MediaError::probe_failure(format!(
                    "Batch inspection workers could not start: {error}"
                )));
                return false;
            }
        };
        self.batch_edit = None;
        self.batch_setup = Some(BatchSetupState {
            origin,
            operation: Operation::AudioMix,
            defaults: None,
            items,
            error: None,
            append_error: None,
        });
        self.batch_inspection = Some(BatchInspectionSession {
            owner: inspection,
            item_ids,
        });
        self.show_batch_setup();
        true
    }

    pub(crate) fn poll_batch_inspection(&mut self) {
        let Some(owner) = self.batch_inspection.as_mut() else {
            return;
        };
        let results = owner.owner.poll();
        let complete = owner.owner.is_complete();
        if results.is_empty() && !complete {
            return;
        }
        let item_ids = &owner.item_ids;
        let capabilities = self.discovery.upscale_capabilities();
        let Some(batch) = self.batch_setup.as_mut() else {
            self.batch_inspection = None;
            return;
        };
        for result in results {
            let Some(id) = item_ids.get(result.index) else {
                continue;
            };
            let Some(item) = batch
                .items
                .iter_mut()
                .find(|item| item.id == *id && matches!(item.source, BatchSourceState::Inspecting))
            else {
                continue;
            };
            match result.result {
                Ok(inspected) => {
                    item.source = BatchSourceState::Ready {
                        source_identity: inspected.source_identity,
                        media: inspected.media,
                    };
                }
                Err(reason) => {
                    item.fail_inspection(reason);
                }
            }
        }
        if complete {
            for item in &mut batch.items {
                if item_ids.contains(&item.id)
                    && matches!(item.source, BatchSourceState::Inspecting)
                {
                    item.fail_inspection(
                        "Media inspection worker stopped unexpectedly.".to_owned(),
                    );
                }
            }
        }
        if batch.defaults.is_none()
            && let Some(media) = batch.items.iter().find_map(|item| match &item.source {
                BatchSourceState::Ready { media, .. } => Some(media.clone()),
                BatchSourceState::Inspecting | BatchSourceState::Failed => None,
            })
        {
            let editor = OperationEditorState::new(0, None, media, batch.operation);
            batch.defaults = batch_defaults_from_editor(&editor);
        }
        refresh_batch_items(batch, capabilities);
        self.refresh_batch_admission();
        if complete {
            self.batch_inspection = None;
        }
    }

    pub(crate) fn append_batch_paths(
        &mut self,
        paths: Vec<PathBuf>,
        wake: impl Fn() + Clone + Send + 'static,
    ) -> bool {
        let Some(tools) = self.discovery.ready().cloned() else {
            return false;
        };
        self.append_batch_paths_with(paths, |paths| {
            BatchInspectionOwner::spawn(tools, paths, wake)
        })
    }

    fn append_batch_paths_with(
        &mut self,
        paths: Vec<PathBuf>,
        spawn: impl FnOnce(Vec<PathBuf>) -> std::io::Result<BatchInspectionOwner>,
    ) -> bool {
        if self.navigation() != NavigationState::BatchSetup
            || paths.is_empty()
            || self.batch_inspection.is_some()
        {
            return false;
        }
        let Some(batch) = self.batch_setup.as_ref() else {
            return false;
        };
        let offset = batch.items.len();
        let owner = match spawn(paths.clone()) {
            Ok(owner) => owner,
            Err(error) => {
                if let Some(batch) = self.batch_setup.as_mut() {
                    batch.error = Some(format!("Additional files could not be inspected: {error}"));
                }
                return false;
            }
        };
        let batch = self.batch_setup.as_mut().expect("batch remains active");
        batch.append_error = None;
        let item_ids = (offset..offset + paths.len()).map(BatchItemId).collect();
        batch.items.extend(
            paths
                .into_iter()
                .enumerate()
                .map(|(index, path)| BatchItem {
                    id: BatchItemId(offset + index),
                    path,
                    included: true,
                    source: BatchSourceState::Inspecting,
                    operation: None,
                    readiness: BatchReadiness::Inspecting,
                    reason: None,
                    overridden: false,
                }),
        );
        batch.error = None;
        self.batch_inspection = Some(BatchInspectionSession { owner, item_ids });
        true
    }

    pub(crate) fn begin_batch_edit(&mut self, target: BatchEditTarget) -> bool {
        if self.navigation() != NavigationState::BatchSetup {
            return false;
        }
        let Some(batch) = self.batch_setup.as_ref() else {
            return false;
        };
        let operation = batch.operation;
        let defaults = batch.defaults.clone();
        let (source_identity, media, request) = match target {
            BatchEditTarget::Defaults => {
                let Some(media) = batch.items.iter().find_map(|item| match &item.source {
                    BatchSourceState::Ready { media, .. } => Some(media.clone()),
                    BatchSourceState::Inspecting | BatchSourceState::Failed => None,
                }) else {
                    return false;
                };
                (None, media, None)
            }
            BatchEditTarget::Item(index) => {
                let Some(item) = batch
                    .items
                    .iter()
                    .find(|item| item.id == index && item.can_edit())
                else {
                    return false;
                };
                let BatchSourceState::Ready {
                    source_identity,
                    media,
                } = &item.source
                else {
                    return false;
                };
                (
                    Some(source_identity.clone()),
                    media.clone(),
                    item.operation.clone(),
                )
            }
        };
        let session_id = self.source.allocate_session_id();
        let mut editor = OperationEditorState::new(session_id, source_identity, media, operation);
        if let Some(request) = request.as_ref() {
            editor.hydrate(request);
        } else if let Some(defaults) = defaults.as_ref() {
            apply_batch_defaults(&mut editor, defaults);
        }
        self.batch_edit = Some(BatchEditState { target, editor });
        self.show_batch_edit();
        true
    }

    pub(crate) fn finish_batch_edit(&mut self) -> bool {
        if !matches!(self.navigation(), NavigationState::BatchEdit) {
            return false;
        }
        self.batch_edit = None;
        self.show_batch_setup();
        true
    }

    pub(crate) fn save_batch_edit(&mut self) -> bool {
        if self.navigation() != NavigationState::BatchEdit {
            return false;
        }
        let capabilities = self.discovery.upscale_capabilities();
        let Some(edit) = self.batch_edit.as_mut() else {
            return false;
        };
        let target = edit.target;
        let Some(batch) = self.batch_setup.as_mut() else {
            return false;
        };
        match target {
            BatchEditTarget::Defaults => {
                let Some(defaults) = batch_defaults_from_editor(&edit.editor) else {
                    return false;
                };
                batch.defaults = Some(defaults);
                refresh_batch_items(batch, capabilities);
            }
            BatchEditTarget::Item(index) => {
                let Some(operation) = edit.editor.validated_request(capabilities) else {
                    return false;
                };
                let Some(item) = batch.items.iter_mut().find(|item| item.id == index) else {
                    return false;
                };
                item.overridden |= item.operation.as_ref() != Some(&operation);
                item.operation = Some(operation);
                item.readiness = BatchReadiness::Ready;
                item.reason = None;
            }
        }
        self.finish_batch_edit()
    }

    pub(crate) fn finish_batch_setup(&mut self) -> bool {
        if self.navigation() != NavigationState::BatchSetup {
            return false;
        }
        let Some(batch) = self.batch_setup.take() else {
            return false;
        };
        self.return_from_batch(batch.origin);
        true
    }

    pub(crate) fn enqueue_batch(&mut self) -> bool {
        if self.navigation() != NavigationState::BatchSetup {
            return false;
        }
        let Some(batch) = self.batch_setup.as_mut() else {
            return false;
        };
        batch.error = None;
        let mut source_ids = Vec::new();
        let mut queued = Vec::new();
        for item in &batch.items {
            if !item.included {
                continue;
            }
            let BatchSourceState::Ready {
                source_identity,
                media,
            } = &item.source
            else {
                batch.error = Some("Every included file must finish inspection.".to_owned());
                return false;
            };
            if item.readiness != BatchReadiness::Ready {
                batch.error = Some("Resolve or exclude every included file.".to_owned());
                return false;
            }
            let Some(operation) = item.operation.as_ref() else {
                batch.error = Some("Every included file needs a valid operation.".to_owned());
                return false;
            };
            source_ids.push(item.id);
            queued.push((source_identity, media, operation));
        }
        if queued.is_empty() {
            batch.error = Some("Include at least one ready file.".to_owned());
            return false;
        }
        match self.queue.enqueue_batch(&queued) {
            Ok(_) => {
                let origin = batch.origin;
                self.complete_batch(origin);
                true
            }
            Err(error) => {
                let message = match &error {
                    BatchEnqueueError::Item { error, .. } => error.to_string(),
                    _ => error.to_string(),
                };
                self.refresh_batch_admission();
                if let Some(batch) = self.batch_setup.as_mut() {
                    // Preserve the precise failure even if the filesystem changed again during refresh.
                    let index = match &error {
                        BatchEnqueueError::Item { index, .. }
                        | BatchEnqueueError::DuplicateDestination { index } => {
                            source_ids.get(*index)
                        }
                        BatchEnqueueError::JobIdExhausted => None,
                    };
                    if let Some(item) =
                        index.and_then(|id| batch.items.iter_mut().find(|item| item.id == *id))
                    {
                        item.readiness = BatchReadiness::NeedsAttention;
                        item.reason = Some(match error {
                            BatchEnqueueError::Item { error, .. } => error.to_string(),
                            _ => message.clone(),
                        });
                    }
                    batch.error = Some(format!("Batch was not queued: {message}"));
                }
                false
            }
        }
    }

    pub(crate) fn select_batch_operation(&mut self, operation: Operation) {
        if self.navigation() != NavigationState::BatchSetup {
            return;
        }
        let capabilities = self.discovery.upscale_capabilities();
        if let Some(batch) = self.batch_setup.as_mut()
            && batch.operation != operation
        {
            batch.operation = operation;
            batch.defaults = None;
            batch.error = None;
            for item in &mut batch.items {
                item.overridden = false;
            }
            if let Some(media) = batch.items.iter().find_map(|item| match &item.source {
                BatchSourceState::Ready { media, .. } => Some(media.clone()),
                BatchSourceState::Inspecting | BatchSourceState::Failed => None,
            }) {
                let editor = OperationEditorState::new(0, None, media, operation);
                batch.defaults = batch_defaults_from_editor(&editor);
            }
            refresh_batch_items(batch, capabilities);
            self.refresh_batch_admission();
        }
    }

    pub(crate) fn toggle_batch_item(&mut self, id: BatchItemId) {
        if self.navigation() != NavigationState::BatchSetup {
            return;
        }
        if let Some(item) = self
            .batch_setup
            .as_mut()
            .and_then(|batch| batch.items.iter_mut().find(|item| item.id == id))
            && item.can_toggle_included()
        {
            item.included = !item.included;
            self.refresh_batch_admission();
        }
    }

    /// Reconcile retained requests when settings, membership or queue reservations change.
    /// This is called by state transitions, never by the renderer.
    pub(crate) fn refresh_batch_admission(&mut self) {
        let Some(batch) = self.batch_setup.as_mut() else {
            return;
        };
        batch.error = None;
        let mut ids = Vec::new();
        let mut candidates = Vec::new();
        let capabilities = self.discovery.settled_capabilities();
        for item in &mut batch.items {
            let (
                BatchSourceState::Ready {
                    source_identity,
                    media,
                },
                Some(operation),
            ) = (&item.source, &mut item.operation)
            else {
                continue;
            };
            // Retain custom requests while discovery changes. Their settings remain editable.
            if let Err(error) = operation.refresh_encoder_capabilities(capabilities) {
                item.readiness = BatchReadiness::NeedsAttention;
                item.reason = Some(error.to_string());
                continue;
            }
            item.readiness = BatchReadiness::Ready;
            item.reason = None;
            if item.included {
                ids.push(item.id);
                candidates.push((source_identity, media, &*operation));
            } else if let Err(error) =
                crate::queue::MediaQueue::validate_admission(source_identity, media, operation)
            {
                item.readiness = BatchReadiness::NeedsAttention;
                item.reason = Some(error.to_string());
            }
        }
        for error in self.queue.scheduler().batch_admission_errors(&candidates) {
            let (index, message) = match error {
                BatchEnqueueError::Item { index, error } => (index, error.to_string()),
                BatchEnqueueError::DuplicateDestination { index } => (index, error.to_string()),
                BatchEnqueueError::JobIdExhausted => continue,
            };
            if let Some(item) = batch.items.iter_mut().find(|item| item.id == ids[index]) {
                item.readiness = BatchReadiness::NeedsAttention;
                item.reason = Some(message);
            }
        }
    }

    pub(crate) fn retire_batch(&mut self) {
        self.batch_inspection = None;
        self.batch_edit = None;
        self.batch_setup = None;
    }

    pub(crate) fn batch_availability(&self) -> Option<BatchAvailability> {
        let batch = self.batch_setup.as_ref()?;
        Some(BatchAvailability {
            can_configure: batch.can_configure(),
            can_add_files: !self.batch_inspecting(),
            can_enqueue: batch.can_enqueue(),
        })
    }

    pub(crate) fn batch_inspecting(&self) -> bool {
        self.batch_inspection.is_some()
    }

    pub(crate) fn set_batch_append_error(&mut self, error: MediaError) {
        if let Some(batch) = self.batch_setup.as_mut() {
            batch.error = None;
            batch.append_error = Some(error);
        }
    }

    pub(crate) fn refresh_batch_capabilities(&mut self) {
        if let Some(batch) = self.batch_setup.as_mut()
            && batch.operation == Operation::VideoUpscale
        {
            refresh_batch_items(batch, self.discovery.upscale_capabilities());
            self.refresh_batch_admission();
        }
    }
}

pub(crate) fn batch_defaults_from_editor(edit: &OperationEditorState) -> Option<BatchDefaults> {
    match edit.active_operation {
        Operation::AudioMix => Some(BatchDefaults::AudioMix {
            container: edit.audio_mix.container(),
            normalization: if edit.audio_mix.normalization_enabled() {
                LoudnessNormalization::Enabled(edit.audio_mix.normalization_targets())
            } else {
                LoudnessNormalization::Off
            },
        }),
        Operation::VideoUpscale => {
            let target = edit.upscale.target();
            let (max_width, max_height) = target.boundary();
            if max_width == 0
                || max_height == 0
                || !max_width.is_multiple_of(2)
                || !max_height.is_multiple_of(2)
            {
                return None;
            }
            Some(BatchDefaults::Upscale {
                target,
                codec: edit.upscale.codec(),
                encoder_settings: edit.upscale.encoder_settings().clone(),
            })
        }
        Operation::Remux => Some(BatchDefaults::Remux {
            container: edit.remux.container(),
        }),
        Operation::AudioExtract => Some(BatchDefaults::AudioExtract {
            format: edit.audio_extract.format(),
        }),
        Operation::FrameCapture => Some(BatchDefaults::FrameCapture {
            timestamp: edit.frame_capture.timestamp_text().parse().ok()?,
            format: edit.frame_capture.format(),
        }),
    }
}

fn apply_batch_defaults(edit: &mut OperationEditorState, defaults: &BatchDefaults) {
    let media = &edit.media;
    match defaults {
        BatchDefaults::AudioMix {
            container,
            normalization,
        } => {
            edit.audio_mix.set_container(media, *container);
            match normalization {
                LoudnessNormalization::Off => {
                    edit.audio_mix.set_normalization_enabled(false);
                }
                LoudnessNormalization::Enabled(targets) => {
                    edit.audio_mix.set_normalization_enabled(true);
                    let _ = edit.audio_mix.set_normalization_targets(*targets);
                }
            }
        }
        BatchDefaults::Upscale {
            target,
            codec,
            encoder_settings,
        } => {
            edit.upscale.set_target(media, *target);
            edit.upscale.set_codec(*codec);
            edit.upscale
                .set_encoder_selection(encoder_settings.selection());
            for codec in [VideoCodec::H264, VideoCodec::H265] {
                edit.upscale
                    .set_cpu_quality(codec, encoder_settings.cpu_quality(codec));
                edit.upscale
                    .set_nvenc_quality(codec, encoder_settings.nvenc_quality(codec));
            }
        }
        BatchDefaults::Remux { container } => {
            edit.remux.set_container(*container);
        }
        BatchDefaults::AudioExtract { format } => {
            edit.audio_extract.set_format(*format);
        }
        BatchDefaults::FrameCapture { timestamp, format } => {
            edit.frame_capture
                .selection
                .set_timestamp_text(media, timestamp.to_string());
            edit.frame_capture.set_format(*format);
        }
    }
}

pub(crate) fn refresh_batch_items(
    batch: &mut BatchSetupState,
    capabilities: Option<UpscaleEncoderCapabilities<'_>>,
) {
    let defaults = batch.defaults.clone();
    for item in &mut batch.items {
        if item.overridden {
            continue;
        }
        let BatchSourceState::Ready { media, .. } = &item.source else {
            continue;
        };
        let media = media.clone();
        item.operation = None;
        item.reason = None;
        let incompatible = match batch.operation {
            Operation::AudioMix | Operation::AudioExtract => media.audio_streams.is_empty(),
            Operation::VideoUpscale | Operation::FrameCapture => media.video_streams.is_empty(),
            Operation::Remux => media.streams.is_empty(),
        };
        if incompatible {
            item.readiness = BatchReadiness::Incompatible;
            item.reason = Some(match batch.operation {
                Operation::AudioMix | Operation::AudioExtract => "No audio tracks.".to_owned(),
                Operation::VideoUpscale | Operation::FrameCapture => "No video track.".to_owned(),
                Operation::Remux => "No media streams.".to_owned(),
            });
            continue;
        }
        if batch.operation == Operation::AudioExtract && media.audio_streams.len() > 1 {
            item.readiness = BatchReadiness::NeedsAttention;
            item.reason = Some("Select one audio track.".to_owned());
            continue;
        }
        let mut editor = OperationEditorState::new(0, None, media, batch.operation);
        if let Some(defaults) = defaults.as_ref()
            && defaults.operation() == batch.operation
        {
            apply_batch_defaults(&mut editor, defaults);
        }
        match editor.validated_request(capabilities) {
            Some(operation) => {
                item.operation = Some(operation);
                item.readiness = BatchReadiness::Ready;
            }
            None => {
                item.readiness = BatchReadiness::NeedsAttention;
                item.reason = Some(
                    match batch.operation {
                        Operation::VideoUpscale
                            if matches!(
                                capabilities,
                                Some(UpscaleEncoderCapabilities::Checking)
                            ) && editor.upscale.encoder_selection()
                                != crate::upscale::model::VideoEncoderSelection::Cpu =>
                        {
                            "Checking NVIDIA NVENC..."
                        }
                        Operation::VideoUpscale => "Shared target is not valid for this source.",
                        Operation::FrameCapture => "Timestamp is outside this source's duration.",
                        Operation::Remux => "Container is not compatible with this source.",
                        Operation::AudioMix => "Audio mix settings are not valid for this source.",
                        Operation::AudioExtract => "Audio extraction settings need review.",
                    }
                    .to_owned(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;

//! Legacy egui composition. The library core owns route and non-rendering state.

mod render;
mod state;
mod workflows;

use std::sync::mpsc::{self, Receiver};

use egui::{Context, Ui};

use crate::app::Application;
use crate::file_dialog::{FileDialogOwner, WindowOwner};
use crate::frame_capture::clipboard::{CopyFrameExtractionOwner, CopyFrameWorker};
use crate::frame_capture::frame_index::FrameIndexQueryOwner;
use crate::frame_capture::preview::PreviewExtractionOwner;
use crate::media::tools::{
    MediaToolsError, cached_video_encoder_capabilities, discover_base_media_tools,
};
use crate::progress_repaint::MediaProgressRepaintOwner;
use crate::settings::AppSettings;
use crate::ui::{fonts, theme};

use state::{
    BatchEditState, BatchInspectionSession, BatchSetupState, FrameCaptureThumbnailLoad,
    FramePreviewDecode, LegacyEditorState, MediaInspection, MediaToolsDiscovery, MediaToolsState,
    QueueEditState, QueueExecutionEvent,
};

pub struct ApolloApp {
    core: Application,
    state: LegacyEditorState,
    media_tools: MediaToolsState,
    media_tools_discovery: Option<MediaToolsDiscovery>,
    queue_edit: Option<QueueEditState>,
    batch_setup: Option<BatchSetupState>,
    batch_edit: Option<BatchEditState>,
    batch_inspection: Option<BatchInspectionSession>,
    queue_event_sender: mpsc::Sender<QueueExecutionEvent>,
    queue_event_receiver: Receiver<QueueExecutionEvent>,
    frame_capture_thumbnail_load: Option<FrameCaptureThumbnailLoad>,
    frame_preview_extraction: PreviewExtractionOwner,
    frame_preview_decode: Option<FramePreviewDecode>,
    frame_index_query: FrameIndexQueryOwner,
    copy_frame_extraction: CopyFrameExtractionOwner,
    copy_frame_worker: Option<CopyFrameWorker>,
    copy_frame_session_id: Option<u64>,
    progress_repaint: MediaProgressRepaintOwner,
    media_inspection: Option<MediaInspection>,
    file_dialog: FileDialogOwner,
    clipboard_owner: Option<isize>,
    settings: AppSettings,
    egui_context: Context,
    next_media_tools_attempt_id: u64,
    next_session_id: u64,
    tools_error_details_open: bool,
}

impl ApolloApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        fonts::install(&creation_context.egui_ctx);
        theme::install(&creation_context.egui_ctx);

        let egui_context = creation_context.egui_ctx.clone();
        let native_window_owner = WindowOwner::from_window(creation_context);
        #[cfg(target_os = "windows")]
        let clipboard_owner = native_window_owner
            .as_ref()
            .ok()
            .copied()
            .map(WindowOwner::native_handle);
        #[cfg(not(target_os = "windows"))]
        let clipboard_owner = None;
        let file_dialog = FileDialogOwner::new(egui_context.clone(), native_window_owner);
        let settings = AppSettings::load(creation_context.storage);
        let mut app =
            Self::with_adapters(egui_context.clone(), file_dialog, clipboard_owner, settings);
        app.start_progressive_media_tools_discovery(
            egui_context,
            discover_base_media_tools,
            cached_video_encoder_capabilities,
        );
        app
    }

    fn with_adapters(
        egui_context: Context,
        file_dialog: FileDialogOwner,
        clipboard_owner: Option<isize>,
        settings: AppSettings,
    ) -> Self {
        let (queue_event_sender, queue_event_receiver) = mpsc::channel();
        Self {
            core: Application::default(),
            state: LegacyEditorState::default(),
            media_tools: MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(
                "Media tool discovery has not started.".to_owned(),
            )),
            media_tools_discovery: None,
            queue_edit: None,
            batch_setup: None,
            batch_edit: None,
            batch_inspection: None,
            queue_event_sender,
            queue_event_receiver,
            frame_capture_thumbnail_load: None,
            frame_preview_extraction: PreviewExtractionOwner::default(),
            frame_preview_decode: None,
            frame_index_query: FrameIndexQueryOwner::default(),
            copy_frame_extraction: CopyFrameExtractionOwner::default(),
            copy_frame_worker: None,
            copy_frame_session_id: None,
            progress_repaint: MediaProgressRepaintOwner::new(egui_context.clone()),
            media_inspection: None,
            file_dialog,
            clipboard_owner,
            settings,
            egui_context,
            next_media_tools_attempt_id: 0,
            next_session_id: 1,
            tools_error_details_open: false,
        }
    }

    fn poll_background_work(&mut self) {
        self.poll_media_tools_discovery();
        self.poll_media_queue();
        if !matches!(&self.media_tools, MediaToolsState::Ready(_)) {
            return;
        }
        self.poll_file_dialog();
        self.poll_media_inspection();
        self.poll_batch_inspection();
        self.poll_frame_capture_preview();
        self.poll_frame_capture_index();
        self.poll_copy_frame();
        self.poll_frame_capture_thumbnail();
    }

    fn shutdown(&mut self) {
        self.cancel_copy_frame_work();
        self.core.media_queue.cancel_queue();
    }
}

impl Drop for ApolloApp {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl eframe::App for ApolloApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show_root(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.save(storage);
    }
}

#[cfg(test)]
mod tests;
